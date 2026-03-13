use async_trait::async_trait;
use futures_util::FutureExt;
use std::any::Any;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::time::timeout;

use crate::CliResult;
use crate::KernelContext;

use super::super::config::LoongClawConfig;
use super::persistence::{format_provider_error_reply, persist_error_turns, persist_success_turns};
use super::runtime::{ConversationRuntime, DefaultConversationRuntime, SessionContext};
use super::turn_engine::{
    AppToolDispatcher, DefaultAppToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnResult,
};
use super::ProviderErrorMode;

use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{build_terminal_finalize_recovery_payload, RECOVERY_EVENT_KIND};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    CreateSessionWithEventRequest, NewSessionRecord, SessionKind, SessionRepository, SessionState,
    TransitionSessionWithEventIfCurrentRequest,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::{
    delegate_cancelled_error, parse_delegate_cancelled_reason, DELEGATE_CANCELLED_EVENT_KIND,
    DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED, DELEGATE_CANCEL_REQUESTED_EVENT_KIND,
};
use crate::tools::runtime_tool_view_for_config;

#[derive(Default)]
pub struct ConversationTurnLoop;

const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";

impl ConversationTurnLoop {
    pub fn new() -> Self {
        Self
    }

    pub async fn handle_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let session_context = SessionContext::root_with_tool_view(
            session_id,
            runtime_tool_view_for_config(&config.tools),
        );
        self.handle_turn_in_session(config, &session_context, user_input, error_mode, kernel_ctx)
            .await
    }

    pub async fn handle_turn_in_session(
        &self,
        config: &LoongClawConfig,
        session_context: &SessionContext,
        user_input: &str,
        error_mode: ProviderErrorMode,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let runtime = DefaultConversationRuntime;
        let app_dispatcher = default_app_tool_dispatcher(config);
        self.handle_turn_with_runtime_and_context(
            config,
            session_context,
            user_input,
            error_mode,
            &runtime,
            &app_dispatcher,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let session_context = runtime.session_context(config, session_id, kernel_ctx)?;
        let app_dispatcher = default_app_tool_dispatcher(config);
        self.handle_turn_with_runtime_and_context(
            config,
            &session_context,
            user_input,
            error_mode,
            runtime,
            &app_dispatcher,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime_and_context<
        R: ConversationRuntime + ?Sized,
        D: AppToolDispatcher + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_context: &SessionContext,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        app_dispatcher: &D,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        #[cfg(feature = "memory-sqlite")]
        ensure_session_registered(config, session_context)?;

        let turn_loop_dispatcher = TurnLoopAppToolDispatcher {
            turn_loop: self,
            config,
            runtime,
            fallback: app_dispatcher,
        };
        let session_id = session_context.session_id.as_str();
        let tool_view = &session_context.tool_view;
        let mut messages =
            runtime.build_messages(config, session_id, true, tool_view, kernel_ctx)?;
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        let raw_tool_output_requested = user_requested_raw_tool_output(user_input);
        let mut last_raw_reply = String::new();
        let policy = TurnLoopPolicy::from_config(config);
        let mut loop_supervisor = ToolLoopSupervisor::default();
        let mut followup_payload_budget = FollowupPayloadBudget::new(
            policy.max_followup_tool_payload_chars,
            policy.max_followup_tool_payload_chars_total,
        );

        for round_index in 0..policy.max_rounds {
            #[cfg(feature = "memory-sqlite")]
            if session_context.parent_session_id.is_some() {
                if let Some(cancel_reason) =
                    load_delegate_child_cancel_request(config, session_context)?
                {
                    return Err(delegate_cancelled_error(&cancel_reason));
                }
            }

            let turn = match runtime.request_turn(config, &messages, tool_view).await {
                Ok(turn) => turn,
                Err(error) => {
                    return match error_mode {
                        ProviderErrorMode::Propagate => Err(error),
                        ProviderErrorMode::InlineMessage => {
                            let synthetic = format_provider_error_reply(&error);
                            persist_error_turns(
                                runtime, session_id, user_input, &synthetic, kernel_ctx,
                            )
                            .await?;
                            Ok(synthetic)
                        }
                    };
                }
            };

            let had_tool_intents = !turn.tool_intents.is_empty();
            let current_tool_signature =
                had_tool_intents.then(|| tool_intent_signature_for_turn(&turn));
            let current_tool_name_signature =
                had_tool_intents.then(|| tool_name_signature(&turn.tool_intents));

            let turn_result = TurnEngine::new(policy.max_tool_steps_per_round)
                .execute_turn_in_context(&turn, session_context, &turn_loop_dispatcher, kernel_ctx)
                .await;
            let loop_supervisor_verdict = if let (Some(signature), Some(name_signature)) = (
                current_tool_signature.as_deref(),
                current_tool_name_signature.as_deref(),
            ) {
                tool_round_outcome(&turn_result).map(|outcome| {
                    loop_supervisor.observe_round(
                        &policy,
                        signature,
                        name_signature,
                        outcome.fingerprint.as_str(),
                        outcome.failed,
                    )
                })
            } else {
                None
            };

            let reply = match turn_result {
                TurnResult::FinalText(tool_text) if had_tool_intents => {
                    let raw_reply =
                        join_non_empty_lines(&[turn.assistant_text.as_str(), tool_text.as_str()]);
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop { reason }) =
                        loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                Some(("tool_result", tool_text.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                tool_text.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
                            );
                            if round_index + 1 < policy.max_rounds {
                                continue;
                            }
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    }
                }
                TurnResult::ToolDenied(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolDenied(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop {
                        reason: loop_reason,
                    }) = loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                loop_reason.as_str(),
                                user_input,
                                Some(("tool_failure", reason.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_failure_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
                            );
                            if round_index + 1 < policy.max_rounds {
                                continue;
                            }
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    }
                }
                TurnResult::ToolError(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolError(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop {
                        reason: loop_reason,
                    }) = loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                loop_reason.as_str(),
                                user_input,
                                Some(("tool_failure", reason.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_failure_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
                            );
                            if round_index + 1 < policy.max_rounds {
                                continue;
                            }
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    }
                }
                other => {
                    compose_assistant_reply(turn.assistant_text.as_str(), had_tool_intents, other)
                }
            };
            persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
            return Ok(reply);
        }

        let reply = if last_raw_reply.is_empty() {
            "agent_loop_round_limit_reached".to_owned()
        } else {
            last_raw_reply
        };
        persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
        Ok(reply)
    }
}

struct TurnLoopAppToolDispatcher<'a, R: ?Sized, D: ?Sized> {
    turn_loop: &'a ConversationTurnLoop,
    config: &'a LoongClawConfig,
    runtime: &'a R,
    fallback: &'a D,
}

#[async_trait]
impl<R, D> AppToolDispatcher for TurnLoopAppToolDispatcher<'_, R, D>
where
    R: ConversationRuntime + ?Sized,
    D: AppToolDispatcher + ?Sized,
{
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loongclaw_contracts::ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        match request.tool_name.as_str() {
            "delegate" => {
                execute_delegate_tool(
                    self.turn_loop,
                    self.config,
                    self.runtime,
                    self.fallback,
                    session_context,
                    request.payload,
                    kernel_ctx,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(session_context, request, kernel_ctx)
                    .await
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn ensure_session_registered(
    config: &LoongClawConfig,
    session_context: &SessionContext,
) -> CliResult<()> {
    let repo = SessionRepository::new(&memory_runtime_config_for(config))?;
    let kind = if session_context.parent_session_id.is_some() {
        SessionKind::DelegateChild
    } else {
        SessionKind::Root
    };
    let _ = repo.ensure_session(NewSessionRecord {
        session_id: session_context.session_id.clone(),
        kind,
        parent_session_id: session_context.parent_session_id.clone(),
        label: None,
        state: SessionState::Ready,
    })?;
    Ok(())
}

fn default_app_tool_dispatcher(config: &LoongClawConfig) -> DefaultAppToolDispatcher {
    #[cfg(feature = "memory-sqlite")]
    {
        return DefaultAppToolDispatcher::production(
            memory_runtime_config_for(config),
            config.tools.clone(),
        );
    }
    #[cfg(not(feature = "memory-sqlite"))]
    DefaultAppToolDispatcher::new(memory_runtime_config_for(config), config.tools.clone())
}

fn memory_runtime_config_for(config: &LoongClawConfig) -> MemoryRuntimeConfig {
    MemoryRuntimeConfig {
        sqlite_path: Some(config.memory.resolved_sqlite_path()),
    }
}

#[cfg(feature = "memory-sqlite")]
pub async fn run_delegate_child_turn(
    turn_loop: &ConversationTurnLoop,
    config: &LoongClawConfig,
    child_session_id: &str,
    user_input: &str,
    timeout_seconds: u64,
    kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    let runtime = DefaultConversationRuntime;
    let app_dispatcher = default_app_tool_dispatcher(config);
    run_delegate_child_turn_with_runtime(
        turn_loop,
        config,
        &runtime,
        &app_dispatcher,
        child_session_id,
        user_input,
        timeout_seconds,
        kernel_ctx,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
pub async fn run_delegate_child_turn_with_runtime<R, D>(
    turn_loop: &ConversationTurnLoop,
    config: &LoongClawConfig,
    runtime: &R,
    fallback: &D,
    child_session_id: &str,
    user_input: &str,
    timeout_seconds: u64,
    kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String>
where
    R: ConversationRuntime + ?Sized,
    D: AppToolDispatcher + ?Sized,
{
    let repo = SessionRepository::new(&memory_runtime_config_for(config))?;
    let child_execution = load_delegate_child_execution_context(&repo, config, child_session_id)?;

    if repo
        .transition_session_with_event_if_current(
            child_session_id,
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Ready,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some(child_execution.parent_session_id.clone()),
                event_payload_json: json!({
                    "task": user_input,
                    "label": child_execution.child_label.clone(),
                    "timeout_seconds": timeout_seconds,
                }),
            },
        )?
        .is_none()
    {
        let latest_child_session = repo
            .load_session(child_session_id)?
            .ok_or_else(|| format!("delegate child session `{child_session_id}` not found"))?;
        return Err(format!(
            "delegate child session `{child_session_id}` is not runnable from state `{}`",
            latest_child_session.state.as_str()
        ));
    }

    run_started_delegate_child_turn_with_runtime(
        turn_loop,
        config,
        runtime,
        fallback,
        child_session_id,
        user_input,
        timeout_seconds,
        kernel_ctx,
        child_execution,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
struct DelegateChildExecutionContext {
    parent_session_id: String,
    child_label: Option<String>,
    child_can_delegate: bool,
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_child_execution_context(
    repo: &SessionRepository,
    config: &LoongClawConfig,
    child_session_id: &str,
) -> Result<DelegateChildExecutionContext, String> {
    let child_session = repo
        .load_session(child_session_id)?
        .ok_or_else(|| format!("delegate child session `{child_session_id}` not found"))?;
    if child_session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session `{child_session_id}` is not a delegate child session"
        ));
    }
    let parent_session_id = child_session.parent_session_id.clone().ok_or_else(|| {
        format!("delegate child session `{child_session_id}` is missing parent_session_id")
    })?;
    let child_label = child_session.label.clone();
    let child_depth = repo.session_lineage_depth(child_session_id)?;
    let child_can_delegate = child_depth < config.tools.delegate.max_depth;

    Ok(DelegateChildExecutionContext {
        parent_session_id,
        child_label,
        child_can_delegate,
    })
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_child_cancel_request(
    config: &LoongClawConfig,
    session_context: &SessionContext,
) -> Result<Option<String>, String> {
    let repo = SessionRepository::new(&memory_runtime_config_for(config))?;
    let recent_events = repo.list_recent_events(&session_context.session_id, 1)?;
    let Some(event) = recent_events.last() else {
        return Ok(None);
    };
    if event.event_kind != DELEGATE_CANCEL_REQUESTED_EVENT_KIND {
        return Ok(None);
    }
    let reason = event
        .payload_json
        .get("cancel_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED)
        .to_owned();
    Ok(Some(reason))
}

#[cfg(feature = "memory-sqlite")]
async fn run_started_delegate_child_turn_with_runtime<R, D>(
    turn_loop: &ConversationTurnLoop,
    config: &LoongClawConfig,
    runtime: &R,
    fallback: &D,
    child_session_id: &str,
    user_input: &str,
    timeout_seconds: u64,
    kernel_ctx: Option<&KernelContext>,
    child_execution: DelegateChildExecutionContext,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String>
where
    R: ConversationRuntime + ?Sized,
    D: AppToolDispatcher + ?Sized,
{
    let repo = SessionRepository::new(&memory_runtime_config_for(config))?;
    let child_context = SessionContext::child(
        child_session_id.to_owned(),
        child_execution.parent_session_id.clone(),
        crate::tools::delegate_child_tool_view_for_config_with_delegate(
            &config.tools,
            child_execution.child_can_delegate,
        ),
    );
    let start = Instant::now();
    let child_result = timeout(Duration::from_secs(timeout_seconds), async {
        AssertUnwindSafe(turn_loop.handle_turn_with_runtime_and_context(
            config,
            &child_context,
            user_input,
            ProviderErrorMode::Propagate,
            runtime,
            fallback,
            kernel_ctx,
        ))
        .catch_unwind()
        .await
    })
    .await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match child_result {
        Ok(Ok(Ok(final_output))) => {
            let turn_count = repo
                .load_session_summary(child_session_id)?
                .map(|session| session.turn_count)
                .unwrap_or_default();
            let outcome = crate::tools::delegate::delegate_success_outcome(
                child_session_id.to_owned(),
                child_execution.child_label,
                final_output,
                turn_count,
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some(child_execution.parent_session_id.clone()),
                    event_payload_json: json!({
                        "turn_count": turn_count,
                        "duration_ms": duration_ms,
                    }),
                    outcome_status: outcome.status.clone(),
                    outcome_payload_json: outcome.payload.clone(),
                },
            )?;
            Ok(outcome)
        }
        Ok(Ok(Err(error))) => {
            let outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                child_execution.child_label,
                error.clone(),
                duration_ms,
            );
            let (event_kind, event_payload_json) =
                if let Some(cancel_reason) = parse_delegate_cancelled_reason(&error) {
                    (
                        DELEGATE_CANCELLED_EVENT_KIND.to_owned(),
                        json!({
                            "error": error,
                            "duration_ms": duration_ms,
                            "cancel_reason": cancel_reason,
                            "reference": "running",
                        }),
                    )
                } else {
                    (
                        "delegate_failed".to_owned(),
                        json!({
                            "error": error,
                            "duration_ms": duration_ms,
                        }),
                    )
                };
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(error.clone()),
                    event_kind,
                    actor_session_id: Some(child_execution.parent_session_id.clone()),
                    event_payload_json,
                    outcome_status: outcome.status.clone(),
                    outcome_payload_json: outcome.payload.clone(),
                },
            )?;
            Ok(outcome)
        }
        Ok(Err(panic_payload)) => {
            let panic_error = format_delegate_child_panic(panic_payload);
            let outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                child_execution.child_label,
                panic_error.clone(),
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(panic_error.clone()),
                    event_kind: "delegate_failed".to_owned(),
                    actor_session_id: Some(child_execution.parent_session_id.clone()),
                    event_payload_json: json!({
                        "error": panic_error,
                        "duration_ms": duration_ms,
                    }),
                    outcome_status: outcome.status.clone(),
                    outcome_payload_json: outcome.payload.clone(),
                },
            )?;
            Ok(outcome)
        }
        Err(_) => {
            let timeout_error = "delegate_timeout".to_owned();
            let outcome = crate::tools::delegate::delegate_timeout_outcome(
                child_session_id.to_owned(),
                child_execution.child_label,
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::TimedOut,
                    last_error: Some(timeout_error.clone()),
                    event_kind: "delegate_timed_out".to_owned(),
                    actor_session_id: Some(child_execution.parent_session_id.clone()),
                    event_payload_json: json!({
                        "error": timeout_error,
                        "duration_ms": duration_ms,
                    }),
                    outcome_status: outcome.status.clone(),
                    outcome_payload_json: outcome.payload.clone(),
                },
            )?;
            Ok(outcome)
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn finalize_delegate_child_terminal_with_recovery(
    repo: &SessionRepository,
    child_session_id: &str,
    request: crate::session::repository::FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    let recovery_request = request.clone();
    match repo.finalize_session_terminal(child_session_id, request) {
        Ok(_) => Ok(()),
        Err(finalize_error) => {
            let recovery_error = format!("delegate_terminal_finalize_failed: {finalize_error}");
            match repo.transition_session_with_event_if_current(
                child_session_id,
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Running,
                    next_state: SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: recovery_request.actor_session_id.clone(),
                    event_payload_json: build_terminal_finalize_recovery_payload(
                        &recovery_request,
                        &recovery_error,
                    ),
                },
            ) {
                Ok(Some(_)) => Err(recovery_error),
                Ok(None) => {
                    delegate_terminal_recovery_skipped_error(repo, child_session_id, recovery_error)
                }
                Err(recovery_event_error) => match repo.update_session_state_if_current(
                    child_session_id,
                    SessionState::Running,
                    SessionState::Failed,
                    Some(recovery_error.clone()),
                ) {
                    Ok(Some(_)) => Err(format!(
                        "{recovery_error}; delegate_terminal_recovery_event_failed: {recovery_event_error}"
                    )),
                    Ok(None) => delegate_terminal_recovery_skipped_error(
                        repo,
                        child_session_id,
                        recovery_error,
                    ),
                    Err(mark_error) => Err(format!(
                        "{recovery_error}; delegate_terminal_recovery_failed: {mark_error}"
                    )),
                },
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn delegate_terminal_recovery_skipped_error(
    repo: &SessionRepository,
    child_session_id: &str,
    recovery_error: String,
) -> Result<(), String> {
    let current_state = repo
        .load_session(child_session_id)?
        .map(|session| session.state.as_str().to_owned())
        .unwrap_or_else(|| "missing".to_owned());
    Err(format!(
        "{recovery_error}; delegate_terminal_recovery_skipped_from_state: {current_state}"
    ))
}

#[cfg(feature = "memory-sqlite")]
fn format_delegate_child_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_child_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };
    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_child_panic: {}", *message),
        Err(_) => "delegate_child_panic".to_owned(),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn execute_delegate_tool<R, D>(
    turn_loop: &ConversationTurnLoop,
    config: &LoongClawConfig,
    runtime: &R,
    fallback: &D,
    session_context: &SessionContext,
    payload: Value,
    kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String>
where
    R: ConversationRuntime + ?Sized,
    D: AppToolDispatcher + ?Sized,
{
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let delegate_request = crate::tools::delegate::parse_delegate_request_with_default_timeout(
        &payload,
        config.tools.delegate.timeout_seconds,
    )?;
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let repo = SessionRepository::new(&memory_runtime_config_for(config))?;
    let current_depth = repo.session_lineage_depth(&session_context.session_id)?;
    let next_child_depth = current_depth.saturating_add(1);
    if next_child_depth > config.tools.delegate.max_depth {
        return Err(format!(
            "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {}",
            config.tools.delegate.max_depth
        ));
    }
    let child_label = delegate_request.label.clone();
    let child_can_delegate = next_child_depth < config.tools.delegate.max_depth;

    repo.create_session_with_event(CreateSessionWithEventRequest {
        session: NewSessionRecord {
            session_id: child_session_id.clone(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(session_context.session_id.clone()),
            label: child_label.clone(),
            state: SessionState::Running,
        },
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some(session_context.session_id.clone()),
        event_payload_json: json!({
            "task": delegate_request.task,
            "label": child_label.clone(),
            "timeout_seconds": delegate_request.timeout_seconds,
        }),
    })?;
    run_started_delegate_child_turn_with_runtime(
        turn_loop,
        config,
        runtime,
        fallback,
        &child_session_id,
        &delegate_request.task,
        delegate_request.timeout_seconds,
        kernel_ctx,
        DelegateChildExecutionContext {
            parent_session_id: session_context.session_id.clone(),
            child_label,
            child_can_delegate,
        },
    )
    .await
}

#[cfg(not(feature = "memory-sqlite"))]
async fn execute_delegate_tool<R, D>(
    _turn_loop: &ConversationTurnLoop,
    _config: &LoongClawConfig,
    _runtime: &R,
    _fallback: &D,
    _session_context: &SessionContext,
    _payload: Value,
    _kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String>
where
    R: ConversationRuntime + ?Sized,
    D: AppToolDispatcher + ?Sized,
{
    Err("delegate requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

fn append_tool_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    let bounded_result = followup_payload_budget.truncate_payload("tool_result", tool_result_text);
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_result]\n{bounded_result}"),
    }));
    if let Some(reason) = loop_warning_reason {
        messages.push(json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_prompt(user_input, loop_warning_reason),
    }));
}

fn append_tool_failure_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    let bounded_failure =
        followup_payload_budget.truncate_payload("tool_failure", tool_failure_reason);
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_failure]\n{bounded_failure}"),
    }));
    if let Some(reason) = loop_warning_reason {
        messages.push(json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_prompt(user_input, loop_warning_reason),
    }));
}

fn append_repeated_tool_guard_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<(&str, &str)>,
    followup_payload_budget: &mut FollowupPayloadBudget,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    if let Some((label, text)) = latest_tool_context {
        let bounded = followup_payload_budget.truncate_payload(label, text);
        messages.push(json!({
            "role": "assistant",
            "content": format!("[{label}]\n{bounded}"),
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_loop_guard]\n{reason}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": build_tool_loop_guard_prompt(user_input, reason),
    }));
}

fn build_tool_loop_guard_prompt(user_input: &str, reason: &str) -> String {
    format!("{TOOL_LOOP_GUARD_PROMPT}\n\nLoop guard reason:\n{reason}\n\nOriginal request:\n{user_input}")
}

async fn request_completion_with_raw_fallback<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    messages: &[Value],
    raw_reply: &str,
) -> String {
    match runtime.request_completion(config, messages).await {
        Ok(final_reply) => {
            let trimmed = final_reply.trim();
            if trimmed.is_empty() {
                raw_reply.to_owned()
            } else {
                trimmed.to_owned()
            }
        }
        Err(_) => raw_reply.to_owned(),
    }
}

fn user_requested_raw_tool_output(user_input: &str) -> bool {
    let normalized = user_input.to_ascii_lowercase();
    [
        "raw",
        "json",
        "payload",
        "verbatim",
        "exact output",
        "full output",
        "tool output",
        "[ok]",
    ]
    .iter()
    .any(|signal| normalized.contains(signal))
}

fn build_tool_followup_prompt(user_input: &str, loop_warning_reason: Option<&str>) -> String {
    if let Some(reason) = loop_warning_reason {
        return format!(
            "{TOOL_FOLLOWUP_PROMPT}\n\nLoop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient.\n\nOriginal request:\n{user_input}"
        );
    }
    format!("{TOOL_FOLLOWUP_PROMPT}\n\nOriginal request:\n{user_input}")
}

fn truncate_followup_tool_payload(label: &str, text: &str, max_chars: usize) -> String {
    let normalized = text.trim();
    let total_chars = normalized.chars().count();
    if total_chars <= max_chars {
        return normalized.to_owned();
    }

    let reserved_chars = 80usize;
    let keep_chars = max_chars.saturating_sub(reserved_chars).max(1);
    let truncated = normalized.chars().take(keep_chars).collect::<String>();
    let removed = total_chars.saturating_sub(keep_chars);
    format!("{truncated}\n[{label}_truncated] removed_chars={removed}")
}

#[derive(Debug, Clone)]
struct FollowupPayloadBudget {
    per_round_max_chars: usize,
    remaining_total_chars: usize,
}

impl FollowupPayloadBudget {
    fn new(per_round_max_chars: usize, total_max_chars: usize) -> Self {
        Self {
            per_round_max_chars: per_round_max_chars.max(1),
            remaining_total_chars: total_max_chars,
        }
    }

    fn truncate_payload(&mut self, label: &str, text: &str) -> String {
        let per_round_allowed = self
            .per_round_max_chars
            .min(self.remaining_total_chars.max(1));
        if self.remaining_total_chars == 0 {
            let removed = text.trim().chars().count();
            return format!("[{label}_truncated] removed_chars={removed} budget_exhausted=true");
        }

        let bounded = truncate_followup_tool_payload(label, text, per_round_allowed);
        let normalized = text.trim();
        let total_chars = normalized.chars().count();
        let consumed_chars = if total_chars <= per_round_allowed {
            total_chars
        } else if per_round_allowed > 80 {
            per_round_allowed - 80
        } else {
            per_round_allowed
        };
        self.remaining_total_chars = self.remaining_total_chars.saturating_sub(consumed_chars);
        bounded
    }
}

#[derive(Debug, Clone)]
struct ToolRoundOutcome {
    fingerprint: String,
    failed: bool,
}

fn tool_round_outcome(turn_result: &TurnResult) -> Option<ToolRoundOutcome> {
    match turn_result {
        TurnResult::FinalText(text) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_final_text", text),
            failed: false,
        }),
        TurnResult::ToolDenied(reason) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_denied", reason),
            failed: true,
        }),
        TurnResult::ToolError(reason) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_error", reason),
            failed: true,
        }),
        TurnResult::NeedsApproval(_) | TurnResult::ProviderError(_) => None,
    }
}

fn text_fingerprint(label: &str, text: &str) -> String {
    let normalized = text.trim();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let digest = hasher.finish();
    format!("{label}:{digest:016x}")
}

fn tool_intent_signature_for_turn(turn: &ProviderTurn) -> String {
    tool_intent_signature(&turn.tool_intents)
}

fn tool_intent_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| {
            let args = serde_json::to_string(&intent.args_json)
                .unwrap_or_else(|_| "<invalid_tool_args_json>".to_owned());
            format!("{}:{args}", intent.tool_name.trim())
        })
        .collect::<Vec<_>>()
        .join("||")
}

fn tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

#[derive(Debug, Clone, Copy)]
struct TurnLoopPolicy {
    max_rounds: usize,
    max_tool_steps_per_round: usize,
    max_repeated_tool_call_rounds: usize,
    max_ping_pong_cycles: usize,
    max_same_tool_failure_rounds: usize,
    max_followup_tool_payload_chars: usize,
    max_followup_tool_payload_chars_total: usize,
}

impl TurnLoopPolicy {
    fn from_config(config: &LoongClawConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_rounds: turn_loop.max_rounds.max(1),
            max_tool_steps_per_round: turn_loop.max_tool_steps_per_round.max(1),
            max_repeated_tool_call_rounds: turn_loop.max_repeated_tool_call_rounds.max(1),
            max_ping_pong_cycles: turn_loop.max_ping_pong_cycles.max(1),
            max_same_tool_failure_rounds: turn_loop.max_same_tool_failure_rounds.max(1),
            max_followup_tool_payload_chars: turn_loop.max_followup_tool_payload_chars.max(256),
            max_followup_tool_payload_chars_total: turn_loop
                .max_followup_tool_payload_chars_total
                .max(1),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ToolLoopSupervisor {
    last_pattern: Option<String>,
    last_pattern_streak: usize,
    warned_reason_key: Option<String>,
    recent_rounds: VecDeque<ToolLoopObservation>,
}

#[derive(Debug, Clone)]
enum ToolLoopSupervisorVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

#[derive(Debug, Clone)]
struct ToolLoopObservation {
    pattern: String,
    tool_name_signature: String,
    failed: bool,
}

#[derive(Debug, Clone)]
struct LoopDetectionReason {
    key: String,
    text: String,
}

impl ToolLoopSupervisor {
    const MAX_RECENT_ROUNDS: usize = 24;

    fn observe_round(
        &mut self,
        policy: &TurnLoopPolicy,
        tool_signature: &str,
        tool_name_signature: &str,
        outcome_fingerprint: &str,
        failed: bool,
    ) -> ToolLoopSupervisorVerdict {
        let pattern = format!("{tool_signature}::{outcome_fingerprint}");
        if self.last_pattern.as_deref() == Some(pattern.as_str()) {
            self.last_pattern_streak += 1;
        } else {
            self.last_pattern = Some(pattern.clone());
            self.last_pattern_streak = 1;
        }

        self.recent_rounds.push_back(ToolLoopObservation {
            pattern: pattern.clone(),
            tool_name_signature: tool_name_signature.to_owned(),
            failed,
        });
        if self.recent_rounds.len() > Self::MAX_RECENT_ROUNDS {
            self.recent_rounds.pop_front();
        }

        let detection = self
            .check_no_progress(policy.max_repeated_tool_call_rounds)
            .or_else(|| self.check_ping_pong(policy.max_ping_pong_cycles))
            .or_else(|| self.check_failure_streak(policy.max_same_tool_failure_rounds));

        match detection {
            Some(reason) => {
                if self.warned_reason_key.as_deref() == Some(reason.key.as_str()) {
                    ToolLoopSupervisorVerdict::HardStop {
                        reason: reason.text,
                    }
                } else {
                    self.warned_reason_key = Some(reason.key);
                    ToolLoopSupervisorVerdict::InjectWarning {
                        reason: reason.text,
                    }
                }
            }
            None => {
                self.warned_reason_key = None;
                ToolLoopSupervisorVerdict::Continue
            }
        }
    }

    fn check_no_progress(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let pattern = self.last_pattern.as_deref()?;
        if self.last_pattern_streak <= threshold {
            return None;
        }
        Some(LoopDetectionReason {
            key: format!("no_progress:{pattern}"),
            text: format!(
                "repeated_tool_call_no_progress signature_streak={} threshold={threshold}",
                self.last_pattern_streak
            ),
        })
    }

    fn check_ping_pong(&self, cycles: usize) -> Option<LoopDetectionReason> {
        let minimum_rounds = cycles.saturating_mul(2);
        if cycles == 0 || self.recent_rounds.len() < minimum_rounds {
            return None;
        }

        let tail = self
            .recent_rounds
            .iter()
            .rev()
            .take(minimum_rounds)
            .collect::<Vec<_>>();
        let first = tail.first()?.pattern.as_str();
        let second = tail.get(1)?.pattern.as_str();
        if first == second {
            return None;
        }

        let alternating = tail.iter().enumerate().all(|(index, round)| {
            if index % 2 == 0 {
                round.pattern == first
            } else {
                round.pattern == second
            }
        });
        if !alternating {
            return None;
        }

        let (left, right) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        Some(LoopDetectionReason {
            key: format!("ping_pong:{left}<->{right}"),
            text: format!(
                "ping_pong_tool_patterns cycles={} threshold={cycles}",
                minimum_rounds / 2
            ),
        })
    }

    fn check_failure_streak(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let last = self.recent_rounds.back()?;
        if !last.failed {
            return None;
        }
        let streak = self
            .recent_rounds
            .iter()
            .rev()
            .take_while(|round| {
                round.failed && round.tool_name_signature == last.tool_name_signature
            })
            .count();
        if streak < threshold {
            return None;
        }
        Some(LoopDetectionReason {
            key: format!("failure_streak:{}", last.tool_name_signature),
            text: format!(
                "tool_failure_streak rounds={streak} threshold={threshold} tool={}",
                last.tool_name_signature
            ),
        })
    }
}

fn compose_assistant_reply(
    assistant_preface: &str,
    had_tool_intents: bool,
    turn_result: TurnResult,
) -> String {
    match turn_result {
        TurnResult::FinalText(text) => {
            if had_tool_intents {
                join_non_empty_lines(&[assistant_preface, text.as_str()])
            } else {
                text
            }
        }
        TurnResult::NeedsApproval(reason) => {
            let inline = format!("[tool_approval_required] {reason}");
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
        TurnResult::ToolDenied(reason) => join_non_empty_lines(&[assistant_preface, &reason]),
        TurnResult::ToolError(reason) => join_non_empty_lines(&[assistant_preface, &reason]),
        TurnResult::ProviderError(reason) => {
            let inline = format_provider_error_reply(&reason);
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
    }
}

fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
