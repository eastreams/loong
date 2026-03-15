use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Value, json};

use crate::CliResult;
use crate::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;

use super::super::config::LoongClawConfig;
use super::ProviderErrorMode;
use super::persistence::persist_reply_turns_with_mode;
use super::runtime::{ConversationRuntime, DefaultConversationRuntime};
use super::turn_budget::{TurnRoundBudget, TurnRoundBudgetDecision};
use super::turn_engine::{
    DefaultAppToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnResult, TurnValidation,
};
use super::turn_shared::{
    ProviderTurnRequestAction, ReplyPersistenceMode, ToolDrivenFollowupPayload,
    ToolDrivenReplyBaseDecision, ToolDrivenReplyPhase, build_tool_driven_followup_tail,
    build_tool_loop_guard_tail, decide_provider_turn_request_action,
    request_completion_with_raw_fallback, user_requested_raw_tool_output,
};

#[derive(Default)]
pub struct ConversationTurnLoop;

#[derive(Debug, Clone)]
struct TurnLoopSessionState {
    messages: Vec<Value>,
    raw_tool_output_requested: bool,
    last_raw_reply: String,
    loop_supervisor: ToolLoopSupervisor,
    followup_payload_budget: FollowupPayloadBudget,
}

#[derive(Debug, Clone)]
struct RoundKernelEvaluation {
    assistant_preface: String,
    had_tool_intents: bool,
    turn_result: TurnResult,
    loop_verdict: Option<ToolLoopSupervisorVerdict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoundKernelDecision {
    ContinueWithFollowup(RoundFollowup),
    FinalizeDirect {
        reply: String,
    },
    FinalizeWithCompletionPass {
        raw_reply: String,
        followup: RoundFollowup,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnLoopTerminalAction {
    PersistReply {
        reply: String,
        persistence_mode: ReplyPersistenceMode,
    },
    ReturnError {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoundFollowup {
    Tool {
        assistant_preface: String,
        payload: ToolDrivenFollowupPayload,
        loop_warning_reason: Option<String>,
    },
    Guard {
        assistant_preface: String,
        reason: String,
        latest_tool_payload: Option<ToolDrivenFollowupPayload>,
    },
}

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
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.handle_turn_with_runtime(
            config, session_id, user_input, error_mode, &runtime, kernel_ctx,
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
        let policy = TurnLoopPolicy::from_config(config);
        let session_context = runtime.session_context(config, session_id, kernel_ctx)?;
        let tool_view = session_context.tool_view.clone();
        let app_dispatcher = DefaultAppToolDispatcher::with_config(
            MemoryRuntimeConfig::from_memory_config(&config.memory),
            config.clone(),
        );
        let mut session = initialize_turn_loop_session(
            runtime
                .build_messages(config, session_id, true, &tool_view, kernel_ctx)
                .await?,
            user_input,
            &policy,
        );

        for round_index in 0..policy.max_rounds {
            let turn = match decide_provider_turn_request_action(
                runtime
                    .request_turn(config, &session.messages, &tool_view, kernel_ctx)
                    .await,
                error_mode,
            ) {
                ProviderTurnRequestAction::Continue { turn } => turn,
                ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                    return apply_turn_loop_terminal_action(
                        runtime,
                        session_id,
                        user_input,
                        TurnLoopTerminalAction::PersistReply {
                            reply,
                            persistence_mode: ReplyPersistenceMode::InlineProviderError,
                        },
                        kernel_ctx,
                    )
                    .await;
                }
                ProviderTurnRequestAction::ReturnError { error } => {
                    return apply_turn_loop_terminal_action(
                        runtime,
                        session_id,
                        user_input,
                        TurnLoopTerminalAction::ReturnError { error },
                        kernel_ctx,
                    )
                    .await;
                }
            };

            let evaluation = evaluate_round_kernel(
                config,
                &policy,
                &turn,
                &session_context,
                &app_dispatcher,
                kernel_ctx,
                &mut session.loop_supervisor,
            )
            .await;
            let reply_phase = evaluation.reply_phase(session.raw_tool_output_requested);
            if let Some(raw_reply) = reply_phase.raw_reply() {
                session.last_raw_reply = raw_reply.to_owned();
            }
            let decision = decide_round_kernel_action(
                TurnRoundBudget::for_round_index(round_index, policy.max_rounds),
                evaluation,
                reply_phase,
            );

            if let Some(action) = resolve_round_kernel_terminal_action(
                runtime,
                config,
                &mut session,
                user_input,
                decision,
                kernel_ctx,
            )
            .await?
            {
                return apply_turn_loop_terminal_action(
                    runtime, session_id, user_input, action, kernel_ctx,
                )
                .await;
            }
        }

        apply_turn_loop_terminal_action(
            runtime,
            session_id,
            user_input,
            build_round_limit_terminal_action(session.last_raw_reply.as_str()),
            kernel_ctx,
        )
        .await
    }
}

fn build_round_limit_terminal_action(last_raw_reply: &str) -> TurnLoopTerminalAction {
    TurnLoopTerminalAction::PersistReply {
        persistence_mode: ReplyPersistenceMode::Success,
        reply: if last_raw_reply.is_empty() {
            "agent_loop_round_limit_reached".to_owned()
        } else {
            last_raw_reply.to_owned()
        },
    }
}

async fn resolve_round_kernel_terminal_action<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    session: &mut TurnLoopSessionState,
    user_input: &str,
    decision: RoundKernelDecision,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<Option<TurnLoopTerminalAction>> {
    match decision {
        RoundKernelDecision::ContinueWithFollowup(followup) => {
            append_round_followup_messages(session, user_input, followup);
            Ok(None)
        }
        RoundKernelDecision::FinalizeDirect { reply } => {
            Ok(Some(TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode: ReplyPersistenceMode::Success,
            }))
        }
        RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup,
        } => {
            append_round_followup_messages(session, user_input, followup);
            let reply = request_completion_with_raw_fallback(
                runtime,
                config,
                &session.messages,
                kernel_ctx,
                raw_reply.as_str(),
            )
            .await;
            Ok(Some(TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode: ReplyPersistenceMode::Success,
            }))
        }
    }
}

async fn apply_turn_loop_terminal_action<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    action: TurnLoopTerminalAction,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    match action {
        TurnLoopTerminalAction::PersistReply {
            reply,
            persistence_mode,
        } => {
            persist_reply_turns_with_mode(
                runtime,
                session_id,
                user_input,
                &reply,
                persistence_mode,
                kernel_ctx,
            )
            .await?;
            Ok(reply)
        }
        TurnLoopTerminalAction::ReturnError { error } => Err(error),
    }
}

fn initialize_turn_loop_session(
    mut messages: Vec<Value>,
    user_input: &str,
    policy: &TurnLoopPolicy,
) -> TurnLoopSessionState {
    messages.push(json!({
        "role": "user",
        "content": user_input,
    }));
    TurnLoopSessionState {
        messages,
        raw_tool_output_requested: user_requested_raw_tool_output(user_input),
        last_raw_reply: String::new(),
        loop_supervisor: ToolLoopSupervisor::default(),
        followup_payload_budget: FollowupPayloadBudget::new(
            policy.max_followup_tool_payload_chars,
            policy.max_followup_tool_payload_chars_total,
        ),
    }
}

async fn evaluate_round_kernel(
    config: &LoongClawConfig,
    policy: &TurnLoopPolicy,
    turn: &ProviderTurn,
    session_context: &super::runtime::SessionContext,
    app_dispatcher: &DefaultAppToolDispatcher,
    kernel_ctx: Option<&KernelContext>,
    loop_supervisor: &mut ToolLoopSupervisor,
) -> RoundKernelEvaluation {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let current_tool_signature = had_tool_intents.then(|| tool_intent_signature_for_turn(turn));
    let current_tool_name_signature =
        had_tool_intents.then(|| tool_name_signature(&turn.tool_intents));

    let engine = TurnEngine::with_tool_result_payload_summary_limit(
        policy.max_tool_steps_per_round,
        config
            .conversation
            .tool_result_payload_summary_limit_chars(),
    );
    let turn_result = match engine.validate_turn_in_context(turn, session_context) {
        Ok(TurnValidation::FinalText(text)) => TurnResult::FinalText(text),
        Err(failure) => TurnResult::ToolDenied(failure),
        Ok(TurnValidation::ToolExecutionRequired) => match kernel_ctx {
            Some(kernel_ctx) => {
                engine
                    .execute_turn_in_context(turn, session_context, app_dispatcher, kernel_ctx)
                    .await
            }
            None => TurnResult::policy_denied("no_kernel_context", "no_kernel_context"),
        },
    };
    let loop_verdict = if let (Some(signature), Some(name_signature)) = (
        current_tool_signature.as_deref(),
        current_tool_name_signature.as_deref(),
    ) {
        tool_round_outcome(&turn_result).map(|outcome| {
            loop_supervisor.observe_round(
                policy,
                signature,
                name_signature,
                outcome.fingerprint.as_str(),
                outcome.failed,
            )
        })
    } else {
        None
    };

    RoundKernelEvaluation {
        assistant_preface: turn.assistant_text.clone(),
        had_tool_intents,
        turn_result,
        loop_verdict,
    }
}

impl RoundKernelEvaluation {
    fn reply_phase(&self, raw_tool_output_requested: bool) -> ToolDrivenReplyPhase {
        ToolDrivenReplyPhase::new(
            self.assistant_preface.as_str(),
            self.had_tool_intents,
            raw_tool_output_requested,
            &self.turn_result,
        )
    }

    fn loop_warning_reason(&self) -> Option<String> {
        match self.loop_verdict.as_ref() {
            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => Some(reason.clone()),
            _ => None,
        }
    }

    fn hard_stop_reason(&self) -> Option<String> {
        match self.loop_verdict.as_ref() {
            Some(ToolLoopSupervisorVerdict::HardStop { reason }) => Some(reason.clone()),
            _ => None,
        }
    }
}

fn decide_round_kernel_action(
    round_budget: TurnRoundBudget,
    evaluation: RoundKernelEvaluation,
    reply_phase: ToolDrivenReplyPhase,
) -> RoundKernelDecision {
    let (raw_reply, tool_payload) = match reply_phase.into_decision() {
        ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => {
            return RoundKernelDecision::FinalizeDirect { reply };
        }
        ToolDrivenReplyBaseDecision::RequireFollowup { raw_reply, payload } => (raw_reply, payload),
    };

    if let Some(reason) = evaluation.hard_stop_reason() {
        return RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup: RoundFollowup::Guard {
                assistant_preface: evaluation.assistant_preface,
                reason,
                latest_tool_payload: Some(tool_payload),
            },
        };
    }

    let loop_warning_reason = evaluation.loop_warning_reason();
    let followup = RoundFollowup::Tool {
        assistant_preface: evaluation.assistant_preface,
        payload: tool_payload,
        loop_warning_reason,
    };

    match round_budget.followup_decision() {
        TurnRoundBudgetDecision::ContinueWithFollowup => {
            RoundKernelDecision::ContinueWithFollowup(followup)
        }
        TurnRoundBudgetDecision::FinalizeWithCompletionPass => {
            RoundKernelDecision::FinalizeWithCompletionPass {
                raw_reply,
                followup,
            }
        }
    }
}

fn append_round_followup_messages(
    session: &mut TurnLoopSessionState,
    user_input: &str,
    followup: RoundFollowup,
) {
    match followup {
        RoundFollowup::Tool {
            assistant_preface,
            payload,
            loop_warning_reason,
        } => append_tool_driven_followup_messages(
            &mut session.messages,
            assistant_preface.as_str(),
            &payload,
            user_input,
            &mut session.followup_payload_budget,
            loop_warning_reason.as_deref(),
        ),
        RoundFollowup::Guard {
            assistant_preface,
            reason,
            latest_tool_payload,
        } => append_repeated_tool_guard_followup_messages(
            &mut session.messages,
            assistant_preface.as_str(),
            reason.as_str(),
            user_input,
            latest_tool_payload.as_ref().map(round_tool_payload_context),
            &mut session.followup_payload_budget,
        ),
    }
}

fn round_tool_payload_context(payload: &ToolDrivenFollowupPayload) -> (&'static str, &str) {
    payload.message_context()
}

fn append_tool_driven_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    user_input: &str,
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
) {
    messages.extend(build_tool_driven_followup_tail(
        assistant_preface,
        payload,
        user_input,
        loop_warning_reason,
        |label, text| followup_payload_budget.truncate_payload(label, text),
    ));
}

fn append_repeated_tool_guard_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<(&str, &str)>,
    followup_payload_budget: &mut FollowupPayloadBudget,
) {
    messages.extend(build_tool_loop_guard_tail(
        assistant_preface,
        reason,
        user_input,
        latest_tool_context,
        |label, text| followup_payload_budget.truncate_payload(label, text),
    ));
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
        TurnResult::ProviderError(_) => None,
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
            pattern,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::turn_engine::TurnFailure;

    #[test]
    fn append_tool_driven_followup_messages_adds_truncation_hint_to_user_prompt() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
            },
            "summarize note.md",
            &mut budget,
            None,
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(
            user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT)
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_omits_truncation_hint_in_user_prompt() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolFailure {
                reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
            },
            "summarize note.md",
            &mut budget,
            None,
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(
            !user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT)
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_promotes_external_skill_invoke_into_system_context() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(64, 64);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
            },
            "summarize note.md",
            &mut budget,
            None,
        );

        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "system");
        let system_content = messages[1]["content"]
            .as_str()
            .expect("system content should exist");
        assert!(system_content.contains("Demo Skill"));
        assert!(system_content.contains("Follow the managed skill instruction before answering."));
        assert!(
            !system_content.contains("[tool_result_truncated]"),
            "invoke instructions should not be funneled through followup truncation markers"
        );

        let user_prompt = messages[2]["content"]
            .as_str()
            .expect("user prompt should exist");
        assert!(user_prompt.contains("managed external skill"));
        assert!(user_prompt.contains("Original request:\nsummarize note.md"));
    }

    #[test]
    fn append_tool_driven_followup_messages_keeps_large_external_skill_instructions_intact() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(32, 32);
        let instructions = format!("prefix {}\nsuffix-marker", "x".repeat(512));
        let payload_summary = serde_json::json!({
            "skill_id": "demo-skill",
            "display_name": "Demo Skill",
            "instructions": instructions,
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "external_skills.invoke",
                "tool_call_id": "call-2",
                "payload_summary": payload_summary,
                "payload_chars": 2048,
                "payload_truncated": false
            })
        );

        append_tool_driven_followup_messages(
            &mut messages,
            "",
            &ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "apply the skill",
            &mut budget,
            None,
        );

        let system_content = messages[0]["content"]
            .as_str()
            .expect("system content should exist");
        assert!(
            system_content.contains("suffix-marker"),
            "system context should preserve the tail of large invoke instructions"
        );
    }

    #[test]
    fn decide_round_kernel_action_continues_tool_result_with_warning_before_round_limit() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::InjectWarning {
                reason: "warning".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::ContinueWithFollowup(RoundFollowup::Tool {
            assistant_preface,
            payload: ToolDrivenFollowupPayload::ToolResult { text },
            loop_warning_reason,
        }) = decision
        {
            assert_eq!(assistant_preface, "preface");
            assert_eq!(text, "tool output");
            assert_eq!(loop_warning_reason.as_deref(), Some("warning"));
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_hard_stop_tool_result_uses_completion_pass() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::HardStop {
                reason: "stop".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup:
                RoundFollowup::Guard {
                    assistant_preface,
                    reason,
                    latest_tool_payload: Some(ToolDrivenFollowupPayload::ToolResult { text }),
                },
        } = decision
        {
            assert_eq!(raw_reply, "preface\ntool output");
            assert_eq!(assistant_preface, "preface");
            assert_eq!(reason, "stop");
            assert_eq!(text, "tool output");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_hard_stop_tool_failure_uses_completion_pass() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            turn_result: TurnResult::ToolError(TurnFailure::retryable(
                "tool_failed",
                "tool failure",
            )),
            loop_verdict: Some(ToolLoopSupervisorVerdict::HardStop {
                reason: "stop".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup:
                RoundFollowup::Guard {
                    assistant_preface,
                    reason,
                    latest_tool_payload:
                        Some(ToolDrivenFollowupPayload::ToolFailure {
                            reason: tool_reason,
                        }),
                },
        } = decision
        {
            assert_eq!(raw_reply, "preface\ntool failure");
            assert_eq!(assistant_preface, "preface");
            assert_eq!(reason, "stop");
            assert_eq!(tool_reason, "tool failure");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_raw_mode_finalizes_tool_result_directly() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::InjectWarning {
                reason: "warning".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(true);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeDirect { reply } = decision {
            assert_eq!(reply, "preface\ntool output");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn turn_round_budget_detects_followup_capacity() {
        let first_round = TurnRoundBudget::for_round_index(0, 3);
        let last_round = TurnRoundBudget::for_round_index(2, 3);

        assert_eq!(
            first_round.followup_decision(),
            TurnRoundBudgetDecision::ContinueWithFollowup
        );
        assert_eq!(
            last_round.followup_decision(),
            TurnRoundBudgetDecision::FinalizeWithCompletionPass
        );
    }

    #[test]
    fn decide_provider_turn_request_action_continues_successful_turns() {
        let action = decide_provider_turn_request_action(
            Ok(ProviderTurn {
                assistant_text: "preface".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            }),
            ProviderErrorMode::Propagate,
        );

        match action {
            ProviderTurnRequestAction::Continue { turn } => {
                assert_eq!(turn.assistant_text, "preface");
                assert!(turn.tool_intents.is_empty());
            }
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                panic!("unexpected inline error action: {reply}");
            }
            ProviderTurnRequestAction::ReturnError { error } => {
                panic!("unexpected propagated error action: {error}");
            }
        }
    }

    #[test]
    fn decide_provider_turn_request_action_formats_inline_provider_errors() {
        let action = decide_provider_turn_request_action(
            Err("timeout".to_owned()),
            ProviderErrorMode::InlineMessage,
        );

        match action {
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                assert_eq!(reply, "[provider_error] timeout");
            }
            ProviderTurnRequestAction::Continue { turn } => {
                panic!("unexpected continue action: {turn:?}");
            }
            ProviderTurnRequestAction::ReturnError { error } => {
                panic!("unexpected propagated error action: {error}");
            }
        }
    }

    #[test]
    fn decide_provider_turn_request_action_propagates_provider_errors() {
        let action = decide_provider_turn_request_action(
            Err("timeout".to_owned()),
            ProviderErrorMode::Propagate,
        );

        match action {
            ProviderTurnRequestAction::ReturnError { error } => {
                assert_eq!(error, "timeout");
            }
            ProviderTurnRequestAction::Continue { turn } => {
                panic!("unexpected continue action: {turn:?}");
            }
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                panic!("unexpected inline error action: {reply}");
            }
        }
    }

    #[test]
    fn build_round_limit_terminal_action_prefers_last_raw_reply() {
        let action = build_round_limit_terminal_action("last raw reply");

        match action {
            TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode,
            } => {
                assert_eq!(reply, "last raw reply");
                assert_eq!(persistence_mode, ReplyPersistenceMode::Success);
            }
            TurnLoopTerminalAction::ReturnError { error } => {
                panic!("unexpected propagated error terminal action: {error}");
            }
        }
    }

    #[test]
    fn build_round_limit_terminal_action_uses_synthetic_reply_when_raw_reply_missing() {
        let action = build_round_limit_terminal_action("");

        match action {
            TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode,
            } => {
                assert_eq!(reply, "agent_loop_round_limit_reached");
                assert_eq!(persistence_mode, ReplyPersistenceMode::Success);
            }
            TurnLoopTerminalAction::ReturnError { error } => {
                panic!("unexpected propagated error terminal action: {error}");
            }
        }
    }
}
