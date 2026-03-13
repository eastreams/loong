use std::collections::BTreeSet;

use async_trait::async_trait;
use loongclaw_contracts::{
    AuditEventKind, Capability, ExecutionPlane, MemoryCoreRequest, PlaneTier, ToolCoreRequest,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::CliResult;
use crate::KernelContext;
use crate::acp::{
    AcpConversationTurnEntryDecision, AcpConversationTurnExecutionOutcome,
    AcpConversationTurnOptions, AcpTurnEventSink, evaluate_acp_conversation_turn_entry_for_address,
    execute_acp_conversation_turn_for_address,
};

use super::super::config::LoongClawConfig;
use super::ConversationSessionAddress;
use super::ProviderErrorMode;
use super::analytics::{
    SafeLaneEventSummary, parse_conversation_event, summarize_safe_lane_events,
};
use super::lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
use super::persistence::{
    format_provider_error_reply, persist_acp_runtime_events, persist_conversation_event,
    persist_error_turns, persist_error_turns_raw, persist_success_turns, persist_success_turns_raw,
};
use super::plan_executor::{
    PlanExecutor, PlanNodeError, PlanNodeErrorKind, PlanNodeExecutor, PlanRunFailure, PlanRunStatus,
};
use super::plan_ir::{
    PLAN_GRAPH_VERSION, PlanBudget, PlanEdge, PlanGraph, PlanNode, PlanNodeKind, RiskTier,
};
use super::plan_verifier::{
    PlanVerificationContext, PlanVerificationFailureCode, PlanVerificationPolicy,
    PlanVerificationReport, verify_output,
};
use super::runtime::{ConversationRuntime, DefaultConversationRuntime};
use super::turn_engine::{
    KernelFailureClass, ProviderTurn, ToolIntent, TurnEngine, TurnFailure, TurnFailureKind,
    TurnResult, classify_kernel_error,
};
use super::turn_shared::{
    build_tool_followup_user_prompt, compose_assistant_reply, join_non_empty_lines,
    tool_result_contains_truncation_signal, user_requested_raw_tool_output,
};

#[derive(Default)]
pub struct ConversationTurnCoordinator;

const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "A managed external skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SafeLaneExecutionMetrics {
    rounds_started: u32,
    rounds_succeeded: u32,
    rounds_failed: u32,
    verify_failures: u32,
    replans_triggered: u32,
    total_attempts_used: u64,
    tool_output_result_lines_total: u64,
    tool_output_truncated_result_lines_total: u64,
}

impl SafeLaneExecutionMetrics {
    fn record_tool_output_stats(&mut self, stats: SafeLaneToolOutputStats) {
        self.tool_output_result_lines_total = self
            .tool_output_result_lines_total
            .saturating_add(stats.result_lines as u64);
        self.tool_output_truncated_result_lines_total = self
            .tool_output_truncated_result_lines_total
            .saturating_add(stats.truncated_result_lines as u64);
    }

    fn aggregate_tool_truncation_ratio_milli(self) -> Option<u32> {
        if self.tool_output_result_lines_total == 0 {
            return None;
        }
        Some(
            self.tool_output_truncated_result_lines_total
                .saturating_mul(1000)
                .saturating_div(self.tool_output_result_lines_total)
                .min(u32::MAX as u64) as u32,
        )
    }

    fn as_json(self) -> Value {
        json!({
            "rounds_started": self.rounds_started,
            "rounds_succeeded": self.rounds_succeeded,
            "rounds_failed": self.rounds_failed,
            "verify_failures": self.verify_failures,
            "replans_triggered": self.replans_triggered,
            "total_attempts_used": self.total_attempts_used,
            "tool_output_result_lines_total": self.tool_output_result_lines_total,
            "tool_output_truncated_result_lines_total": self.tool_output_truncated_result_lines_total,
            "tool_output_aggregate_truncation_ratio_milli": self.aggregate_tool_truncation_ratio_milli(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SafeLaneAdaptiveVerifyPolicyState {
    min_anchor_matches: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SafeLaneToolOutputStats {
    output_lines: usize,
    result_lines: usize,
    truncated_result_lines: usize,
}

impl SafeLaneToolOutputStats {
    fn truncation_ratio_milli(self) -> usize {
        if self.result_lines == 0 {
            return 0;
        }
        self.truncated_result_lines
            .saturating_mul(1000)
            .saturating_div(self.result_lines)
    }

    fn as_json(self) -> Value {
        json!({
            "output_lines": self.output_lines,
            "result_lines": self.result_lines,
            "truncated_result_lines": self.truncated_result_lines,
            "any_truncated": self.truncated_result_lines > 0,
            "truncation_ratio_milli": self.truncation_ratio_milli(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeLaneRuntimeHealthSignal {
    severity: &'static str,
    flags: Vec<String>,
}

impl SafeLaneRuntimeHealthSignal {
    fn as_json(&self) -> Value {
        json!({
            "severity": self.severity,
            "flags": self.flags,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct SafeLaneSessionGovernorDecision {
    engaged: bool,
    history_window_turns: usize,
    failed_final_status_events: u32,
    failed_final_status_threshold: u32,
    failed_threshold_triggered: bool,
    backpressure_failure_events: u32,
    backpressure_failure_threshold: u32,
    backpressure_threshold_triggered: bool,
    trend_enabled: bool,
    trend_samples: usize,
    trend_min_samples: usize,
    trend_failure_ewma: Option<f64>,
    trend_failure_ewma_threshold: f64,
    trend_backpressure_ewma: Option<f64>,
    trend_backpressure_ewma_threshold: f64,
    trend_threshold_triggered: bool,
    recovery_success_streak: u32,
    recovery_success_streak_threshold: u32,
    recovery_failure_ewma_threshold: f64,
    recovery_backpressure_ewma_threshold: f64,
    recovery_threshold_triggered: bool,
    force_no_replan: bool,
    forced_node_max_attempts: Option<u8>,
}

impl SafeLaneSessionGovernorDecision {
    fn as_json(self) -> Value {
        json!({
            "engaged": self.engaged,
            "history_window_turns": self.history_window_turns,
            "failed_final_status_events": self.failed_final_status_events,
            "failed_final_status_threshold": self.failed_final_status_threshold,
            "failed_threshold_triggered": self.failed_threshold_triggered,
            "backpressure_failure_events": self.backpressure_failure_events,
            "backpressure_failure_threshold": self.backpressure_failure_threshold,
            "backpressure_threshold_triggered": self.backpressure_threshold_triggered,
            "trend_enabled": self.trend_enabled,
            "trend_samples": self.trend_samples,
            "trend_min_samples": self.trend_min_samples,
            "trend_failure_ewma": self.trend_failure_ewma,
            "trend_failure_ewma_threshold": self.trend_failure_ewma_threshold,
            "trend_backpressure_ewma": self.trend_backpressure_ewma,
            "trend_backpressure_ewma_threshold": self.trend_backpressure_ewma_threshold,
            "trend_threshold_triggered": self.trend_threshold_triggered,
            "recovery_success_streak": self.recovery_success_streak,
            "recovery_success_streak_threshold": self.recovery_success_streak_threshold,
            "recovery_failure_ewma_threshold": self.recovery_failure_ewma_threshold,
            "recovery_backpressure_ewma_threshold": self.recovery_backpressure_ewma_threshold,
            "recovery_threshold_triggered": self.recovery_threshold_triggered,
            "force_no_replan": self.force_no_replan,
            "forced_node_max_attempts": self.forced_node_max_attempts,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SafeLaneGovernorHistorySignals {
    summary: SafeLaneEventSummary,
    final_status_failed_samples: Vec<bool>,
    backpressure_failure_samples: Vec<bool>,
}

impl ConversationTurnCoordinator {
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
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_acp_options(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_address_and_acp_options(
            config,
            &address,
            user_input,
            error_mode,
            acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_acp_event_sink(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_address(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_address_and_acp_event_sink(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_address_and_acp_options(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.handle_turn_with_runtime_and_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
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
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_runtime_and_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime_and_acp_options<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_runtime_and_address_and_acp_options(
            config,
            &address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime_and_acp_event_sink<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_runtime_and_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime_and_address<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_runtime_and_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime_and_address_and_acp_options<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let session_id = address.session_id.as_str();
        match evaluate_acp_conversation_turn_entry_for_address(config, address, acp_options)? {
            AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled => {
                let error = "ACP is disabled by policy (`acp.enabled=false`)".to_owned();
                return match error_mode {
                    ProviderErrorMode::Propagate => Err(error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&error);
                        persist_error_turns_raw(
                            runtime, session_id, user_input, &synthetic, kernel_ctx,
                        )
                        .await?;
                        Ok(synthetic)
                    }
                };
            }
            AcpConversationTurnEntryDecision::RouteViaAcp => {
                return self
                    .handle_turn_via_acp(
                        config,
                        address,
                        user_input,
                        error_mode,
                        runtime,
                        acp_options,
                        kernel_ctx,
                    )
                    .await;
            }
            AcpConversationTurnEntryDecision::StayOnProvider => {}
        }

        runtime.bootstrap(config, session_id, kernel_ctx).await?;
        let assembled_context = runtime
            .build_context(config, session_id, true, kernel_ctx)
            .await?;
        let mut messages = assembled_context.messages;
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        let lane_policy = lane_policy_from_config(config);
        let lane_decision = if config.conversation.hybrid_lane_enabled {
            lane_policy.decide(user_input)
        } else {
            disabled_lane_decision(user_input)
        };
        let max_tool_steps = match lane_decision.lane {
            ExecutionLane::Fast => config.conversation.fast_lane_max_tool_steps(),
            ExecutionLane::Safe => config.conversation.safe_lane_max_tool_steps(),
        };

        let provider_result = runtime.request_turn(config, &messages, kernel_ctx).await;
        match provider_result {
            Ok(turn) => {
                let had_tool_intents = !turn.tool_intents.is_empty();
                let raw_tool_output_requested = user_requested_raw_tool_output(user_input);
                let turn_result = if should_use_safe_lane_plan_path(config, &lane_decision, &turn) {
                    execute_turn_with_safe_lane_plan(
                        config,
                        runtime,
                        session_id,
                        &lane_decision,
                        &turn,
                        kernel_ctx,
                    )
                    .await
                } else {
                    TurnEngine::with_tool_result_payload_summary_limit(
                        max_tool_steps,
                        config
                            .conversation
                            .tool_result_payload_summary_limit_chars(),
                    )
                    .execute_turn(&turn, kernel_ctx)
                    .await
                };
                #[allow(clippy::wildcard_enum_match_arm)]
                let reply = match turn_result {
                    TurnResult::FinalText(tool_text) if had_tool_intents => {
                        let raw_reply = join_non_empty_lines(&[
                            turn.assistant_text.as_str(),
                            tool_text.as_str(),
                        ]);
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            let follow_up_messages = build_tool_followup_messages(
                                &messages,
                                turn.assistant_text.as_str(),
                                tool_text.as_str(),
                                user_input,
                            );
                            match runtime
                                .request_completion(config, &follow_up_messages, kernel_ctx)
                                .await
                            {
                                Ok(final_reply) => {
                                    let trimmed = final_reply.trim();
                                    if trimmed.is_empty() {
                                        raw_reply
                                    } else {
                                        trimmed.to_owned()
                                    }
                                }
                                Err(_) => raw_reply,
                            }
                        }
                    }
                    TurnResult::ToolDenied(failure)
                        if had_tool_intents && !raw_tool_output_requested =>
                    {
                        let raw_reply = compose_assistant_reply(
                            turn.assistant_text.as_str(),
                            had_tool_intents,
                            TurnResult::ToolDenied(failure.clone()),
                        );
                        let follow_up_messages = build_tool_failure_followup_messages(
                            &messages,
                            turn.assistant_text.as_str(),
                            failure.reason.as_str(),
                            user_input,
                        );
                        match runtime
                            .request_completion(config, &follow_up_messages, kernel_ctx)
                            .await
                        {
                            Ok(final_reply) => {
                                let trimmed = final_reply.trim();
                                if trimmed.is_empty() {
                                    raw_reply
                                } else {
                                    trimmed.to_owned()
                                }
                            }
                            Err(_) => raw_reply,
                        }
                    }
                    TurnResult::ToolError(failure)
                        if had_tool_intents && !raw_tool_output_requested =>
                    {
                        let raw_reply = compose_assistant_reply(
                            turn.assistant_text.as_str(),
                            had_tool_intents,
                            TurnResult::ToolError(failure.clone()),
                        );
                        let follow_up_messages = build_tool_failure_followup_messages(
                            &messages,
                            turn.assistant_text.as_str(),
                            failure.reason.as_str(),
                            user_input,
                        );
                        match runtime
                            .request_completion(config, &follow_up_messages, kernel_ctx)
                            .await
                        {
                            Ok(final_reply) => {
                                let trimmed = final_reply.trim();
                                if trimmed.is_empty() {
                                    raw_reply
                                } else {
                                    trimmed.to_owned()
                                }
                            }
                            Err(_) => raw_reply,
                        }
                    }
                    other => compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        other,
                    ),
                };
                persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
                let mut after_turn_messages = messages.clone();
                after_turn_messages.push(json!({
                    "role": "assistant",
                    "content": reply,
                }));
                runtime
                    .after_turn(
                        session_id,
                        user_input,
                        &reply,
                        &after_turn_messages,
                        kernel_ctx,
                    )
                    .await?;
                maybe_compact_context(
                    config,
                    runtime,
                    session_id,
                    &after_turn_messages,
                    assembled_context.estimated_tokens,
                    kernel_ctx,
                )
                .await?;
                Ok(reply)
            }
            Err(error) => match error_mode {
                ProviderErrorMode::Propagate => Err(error),
                ProviderErrorMode::InlineMessage => {
                    let synthetic = format_provider_error_reply(&error);
                    persist_error_turns(runtime, session_id, user_input, &synthetic, kernel_ctx)
                        .await?;
                    let mut after_turn_messages = messages.clone();
                    after_turn_messages.push(json!({
                        "role": "assistant",
                        "content": synthetic,
                    }));
                    runtime
                        .after_turn(
                            session_id,
                            user_input,
                            &synthetic,
                            &after_turn_messages,
                            kernel_ctx,
                        )
                        .await?;
                    maybe_compact_context(
                        config,
                        runtime,
                        session_id,
                        &after_turn_messages,
                        assembled_context.estimated_tokens,
                        kernel_ctx,
                    )
                    .await?;
                    Ok(synthetic)
                }
            },
        }
    }

    pub async fn handle_turn_with_runtime_and_address_and_acp_event_sink<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_runtime_and_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
        )
        .await
    }

    async fn handle_turn_via_acp<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let session_id = address.session_id.as_str();
        let executed =
            execute_acp_conversation_turn_for_address(config, address, user_input, acp_options)
                .await?;
        let persistence_context = &executed.persistence_context;

        match executed.outcome {
            AcpConversationTurnExecutionOutcome::Succeeded(success) => {
                let reply = success.result.output_text.clone();
                persist_success_turns_raw(runtime, session_id, user_input, &reply, kernel_ctx)
                    .await?;
                if config.acp.emit_runtime_events {
                    let _ = persist_acp_runtime_events(
                        runtime,
                        session_id,
                        persistence_context,
                        &success.runtime_events,
                        Some(&success.result),
                        None,
                        kernel_ctx,
                    )
                    .await;
                }
                Ok(reply)
            }
            AcpConversationTurnExecutionOutcome::Failed(failure) => {
                if config.acp.emit_runtime_events {
                    let _ = persist_acp_runtime_events(
                        runtime,
                        session_id,
                        persistence_context,
                        &failure.runtime_events,
                        None,
                        Some(failure.error.as_str()),
                        kernel_ctx,
                    )
                    .await;
                }
                match error_mode {
                    ProviderErrorMode::Propagate => Err(failure.error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&failure.error);
                        persist_error_turns_raw(
                            runtime, session_id, user_input, &synthetic, kernel_ctx,
                        )
                        .await?;
                        Ok(synthetic)
                    }
                }
            }
        }
    }
}

async fn maybe_compact_context<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    messages: &[Value],
    estimated_tokens: Option<usize>,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    let estimated_tokens = estimated_tokens.or_else(|| estimate_tokens(messages));
    if !config
        .conversation
        .should_compact_with_estimate(messages.len(), estimated_tokens)
    {
        return Ok(());
    }

    match runtime
        .compact_context(config, session_id, messages, kernel_ctx)
        .await
    {
        Ok(()) => Ok(()),
        Err(_error) if config.conversation.compaction_fail_open() => Ok(()),
        Err(error) => Err(error),
    }
}

fn estimate_tokens(messages: &[Value]) -> Option<usize> {
    if messages.is_empty() {
        return Some(0);
    }

    let estimated = messages.iter().fold(0usize, |acc, message| {
        let role_chars = message
            .get("role")
            .map_or(0usize, |value| value.to_string().chars().count());
        let content_chars = message
            .get("content")
            .map_or(0usize, |value| value.to_string().chars().count());
        let token_estimate = (role_chars + content_chars).div_ceil(4) + 4;
        acc.saturating_add(token_estimate)
    });

    Some(estimated)
}

fn lane_policy_from_config(config: &LoongClawConfig) -> LaneArbiterPolicy {
    let normalized_keywords = config.conversation.normalized_high_risk_keywords();
    let high_risk_keywords = if normalized_keywords.is_empty() {
        LaneArbiterPolicy::default().high_risk_keywords
    } else {
        normalized_keywords.into_iter().collect::<BTreeSet<_>>()
    };

    LaneArbiterPolicy {
        safe_lane_risk_threshold: config.conversation.safe_lane_risk_threshold,
        safe_lane_complexity_threshold: config.conversation.safe_lane_complexity_threshold,
        fast_lane_max_input_chars: config.conversation.fast_lane_max_input_chars,
        high_risk_keywords,
    }
}

fn disabled_lane_decision(user_input: &str) -> LaneDecision {
    LaneDecision {
        lane: ExecutionLane::Fast,
        risk_score: 0,
        complexity_score: 0,
        reasons: vec![format!(
            "hybrid_lane_disabled chars={}",
            user_input.chars().count()
        )],
    }
}

fn should_use_safe_lane_plan_path(
    config: &LoongClawConfig,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
) -> bool {
    config.conversation.safe_lane_plan_execution_enabled
        && matches!(lane_decision.lane, ExecutionLane::Safe)
        && !turn.tool_intents.is_empty()
}

async fn execute_turn_with_safe_lane_plan<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    kernel_ctx: Option<&KernelContext>,
) -> TurnResult {
    let governor_history_signals =
        load_safe_lane_history_signals_for_governor(config, session_id, kernel_ctx).await;
    let governor = decide_safe_lane_session_governor(config, &governor_history_signals);

    emit_safe_lane_event(
        config,
        runtime,
        session_id,
        "lane_selected",
        json!({
            "lane": "safe",
            "risk_score": lane_decision.risk_score,
            "complexity_score": lane_decision.complexity_score,
            "reasons": lane_decision.reasons.clone(),
            "tool_intents": turn.tool_intents.len(),
            "session_governor": governor.as_json(),
        }),
        kernel_ctx,
    )
    .await;

    let mut round = 0u8;
    let max_rounds = if governor.force_no_replan {
        0
    } else {
        config.conversation.safe_lane_replan_max_rounds
    };
    let mut tool_node_max_attempts = config.conversation.safe_lane_node_max_attempts.max(1);
    if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
        tool_node_max_attempts = tool_node_max_attempts.min(forced_node_max_attempts.max(1));
    }
    let mut max_node_attempts = config
        .conversation
        .safe_lane_replan_max_node_attempts
        .max(tool_node_max_attempts);
    if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
        max_node_attempts = max_node_attempts.min(forced_node_max_attempts.max(1));
    }
    let mut plan_start_tool_index = 0usize;
    let mut seed_tool_outputs = Vec::new();
    let mut metrics = SafeLaneExecutionMetrics::default();
    let mut adaptive_verify_policy = SafeLaneAdaptiveVerifyPolicyState::default();

    loop {
        let next_min_anchor_matches =
            compute_safe_lane_verify_min_anchor_matches(config, metrics.verify_failures);
        if next_min_anchor_matches != adaptive_verify_policy.min_anchor_matches {
            adaptive_verify_policy.min_anchor_matches = next_min_anchor_matches;
            if adaptive_verify_policy.min_anchor_matches > 0 {
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "verify_policy_adjusted",
                    json!({
                        "round": round,
                        "policy": "adaptive_anchor_escalation",
                        "min_anchor_matches": adaptive_verify_policy.min_anchor_matches,
                        "verify_failures": metrics.verify_failures,
                        "escalation_after_failures": config
                            .conversation
                            .safe_lane_verify_anchor_escalation_after_failures(),
                        "metrics": metrics.as_json(),
                    }),
                    kernel_ctx,
                )
                .await;
            }
        }

        metrics.rounds_started = metrics.rounds_started.saturating_add(1);
        emit_safe_lane_event(
            config,
            runtime,
            session_id,
            "plan_round_started",
            json!({
                "round": round,
                "start_tool_index": plan_start_tool_index,
                "tool_node_max_attempts": tool_node_max_attempts,
                "effective_max_rounds": max_rounds,
                "effective_max_node_attempts": max_node_attempts,
                "verify_min_anchor_matches": adaptive_verify_policy.min_anchor_matches,
                "session_governor": governor.as_json(),
                "metrics": metrics.as_json(),
            }),
            kernel_ctx,
        )
        .await;

        let plan = build_safe_lane_plan_graph(
            config,
            lane_decision,
            turn,
            tool_node_max_attempts,
            plan_start_tool_index,
        );
        let executor = SafeLanePlanNodeExecutor::new(
            turn.tool_intents.as_slice(),
            kernel_ctx,
            config.conversation.safe_lane_verify_output_non_empty,
            seed_tool_outputs.clone(),
            config
                .conversation
                .tool_result_payload_summary_limit_chars(),
        );
        let report = PlanExecutor::execute(&plan, &executor).await;
        metrics.total_attempts_used = metrics
            .total_attempts_used
            .saturating_add(report.attempts_used as u64);
        let round_tool_outputs = executor.tool_outputs_snapshot().await;
        let round_tool_output_stats =
            summarize_safe_lane_tool_output_stats(round_tool_outputs.as_slice());
        metrics.record_tool_output_stats(round_tool_output_stats);

        match report.status {
            PlanRunStatus::Succeeded => {
                metrics.rounds_succeeded = metrics.rounds_succeeded.saturating_add(1);
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": round,
                        "status": "succeeded",
                        "attempts_used": report.attempts_used,
                        "elapsed_ms": report.elapsed_ms,
                        "tool_output_stats": round_tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": metrics.as_json(),
                    }),
                    kernel_ctx,
                )
                .await;
                let tool_output = round_tool_outputs.join("\n");
                let verify_report = verify_safe_lane_final_output(
                    config,
                    tool_output.as_str(),
                    turn.tool_intents.as_slice(),
                    adaptive_verify_policy,
                );
                if verify_report.passed {
                    {
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "final_status",
                            json!({
                                "status": "succeeded",
                                "round": round,
                                "tool_output_stats": round_tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": metrics.as_json(),
                            }),
                            kernel_ctx,
                        )
                        .await;
                        return TurnResult::FinalText(tool_output);
                    }
                } else {
                    let verify_error = verify_report.failure_reasons.join(",");
                    let failure_codes = verify_report
                        .failure_codes
                        .iter()
                        .map(format_verification_failure_code)
                        .collect::<Vec<_>>();
                    let retryable_verify_failure =
                        should_replan_for_verification_failure(&verify_report);
                    let verify_failure = turn_failure_from_verify_failure(
                        verify_error.as_str(),
                        retryable_verify_failure,
                    );
                    metrics.verify_failures = metrics.verify_failures.saturating_add(1);
                    let verify_route = apply_safe_lane_backpressure_guard(
                        config,
                        route_safe_lane_failure(&verify_failure, round, max_rounds),
                        metrics,
                    );
                    let verify_route =
                        apply_safe_lane_session_governor_route_override(verify_route, governor);
                    {
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "verify_failed",
                            json!({
                                "round": round,
                                "error": verify_error.clone(),
                                "failure_codes": failure_codes,
                                "retryable": retryable_verify_failure,
                                "failure_kind": format_turn_failure_kind(verify_failure.kind),
                                "failure_code": verify_failure.code.clone(),
                                "failure_retryable": verify_failure.retryable,
                                "route_decision": format_safe_lane_route_decision(verify_route.decision),
                                "route_reason": verify_route.reason,
                                "tool_output_stats": round_tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": metrics.as_json(),
                            }),
                            kernel_ctx,
                        )
                        .await;
                        if matches!(
                            verify_route.decision,
                            SafeLaneFailureRouteDecision::Terminal
                        ) {
                            let terminal_failure = terminal_turn_failure_from_verify_failure(
                                verify_error.as_str(),
                                retryable_verify_failure,
                                verify_route.reason,
                            );
                            emit_safe_lane_event(
                                config,
                                runtime,
                                session_id,
                                "final_status",
                                json!({
                                    "status": "failed",
                                    "round": round,
                                    "failure": summarize_verify_terminal_reason(verify_route.reason),
                                    "failure_kind": format_turn_failure_kind(terminal_failure.kind),
                                    "failure_code": terminal_failure.code.clone(),
                                    "failure_retryable": terminal_failure.retryable,
                                    "route_decision": format_safe_lane_route_decision(verify_route.decision),
                                    "route_reason": verify_route.reason,
                                    "tool_output_stats": round_tool_output_stats.as_json(),
                                    "health_signal": derive_safe_lane_runtime_health_signal(
                                        config,
                                        metrics,
                                        true,
                                        Some(terminal_failure.code.as_str()),
                                    )
                                    .as_json(),
                                    "metrics": metrics.as_json(),
                                }),
                                kernel_ctx,
                            )
                            .await;
                            return TurnResult::ToolError(terminal_failure);
                        }
                        metrics.replans_triggered = metrics.replans_triggered.saturating_add(1);
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "replan_triggered",
                            json!({
                                "round": round,
                                "reason": "verify_failed",
                                "detail": verify_error,
                                "route_decision": format_safe_lane_route_decision(verify_route.decision),
                                "route_reason": verify_route.reason,
                                "tool_output_stats": round_tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": metrics.as_json(),
                            }),
                            kernel_ctx,
                        )
                        .await;
                    }
                }
            }
            PlanRunStatus::Failed(failure) => {
                metrics.rounds_failed = metrics.rounds_failed.saturating_add(1);
                let round_failure_meta = turn_failure_from_plan_failure(&failure);
                let route = apply_safe_lane_backpressure_guard(
                    config,
                    route_safe_lane_failure(&round_failure_meta, round, max_rounds),
                    metrics,
                );
                let route = apply_safe_lane_session_governor_route_override(route, governor);
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": round,
                        "status": "failed",
                        "attempts_used": report.attempts_used,
                        "elapsed_ms": report.elapsed_ms,
                        "failure": summarize_plan_failure(&failure),
                        "failure_kind": format_turn_failure_kind(round_failure_meta.kind),
                        "failure_code": round_failure_meta.code.clone(),
                        "failure_retryable": round_failure_meta.retryable,
                        "route_decision": format_safe_lane_route_decision(route.decision),
                        "route_reason": route.reason,
                        "tool_output_stats": round_tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": metrics.as_json(),
                    }),
                    kernel_ctx,
                )
                .await;
                if matches!(route.decision, SafeLaneFailureRouteDecision::Replan) {
                    let (next_start_tool_index, next_seed_outputs) =
                        derive_replan_cursor(&failure, &executor, turn.tool_intents.len()).await;
                    plan_start_tool_index = next_start_tool_index;
                    seed_tool_outputs = next_seed_outputs;
                    metrics.replans_triggered = metrics.replans_triggered.saturating_add(1);
                    emit_safe_lane_event(
                        config,
                        runtime,
                        session_id,
                        "replan_triggered",
                        json!({
                            "round": round,
                            "reason": summarize_plan_failure(&failure),
                            "restart_tool_index": plan_start_tool_index,
                            "seeded_outputs": seed_tool_outputs.len(),
                            "route_decision": format_safe_lane_route_decision(route.decision),
                            "route_reason": route.reason,
                            "tool_output_stats": round_tool_output_stats.as_json(),
                            "health_signal": derive_safe_lane_runtime_health_signal(
                                config,
                                metrics,
                                false,
                                None,
                            )
                            .as_json(),
                            "metrics": metrics.as_json(),
                        }),
                        kernel_ctx,
                    )
                    .await;
                } else {
                    let terminal_result =
                        terminal_turn_result_from_plan_failure_with_route(failure.clone(), route);
                    let failure_meta = terminal_result.failure();
                    emit_safe_lane_event(
                        config,
                        runtime,
                        session_id,
                        "final_status",
                        json!({
                            "status": "failed",
                            "round": round,
                            "failure": summarize_plan_failure(&failure),
                            "failure_kind": failure_meta
                                .map(|failure| format_turn_failure_kind(failure.kind)),
                            "failure_code": failure_meta.map(|failure| failure.code.clone()),
                            "failure_retryable": failure_meta.map(|failure| failure.retryable),
                            "route_decision": format_safe_lane_route_decision(route.decision),
                            "route_reason": route.reason,
                            "tool_output_stats": round_tool_output_stats.as_json(),
                            "health_signal": derive_safe_lane_runtime_health_signal(
                                config,
                                metrics,
                                true,
                                failure_meta.map(|failure| failure.code.as_str()),
                            )
                            .as_json(),
                            "metrics": metrics.as_json(),
                        }),
                        kernel_ctx,
                    )
                    .await;
                    return terminal_result;
                }
            }
        }

        round = round.saturating_add(1);
        tool_node_max_attempts = tool_node_max_attempts
            .saturating_add(1)
            .min(max_node_attempts)
            .max(1);
    }
}

async fn emit_safe_lane_event<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    event_name: &str,
    payload: Value,
    kernel_ctx: Option<&KernelContext>,
) {
    if !should_emit_safe_lane_event(config, event_name, &payload) {
        return;
    }
    let _ = persist_conversation_event(runtime, session_id, event_name, payload, kernel_ctx).await;
    if let Some(ctx) = kernel_ctx {
        let _ = ctx.kernel.record_audit_event(
            Some(ctx.agent_id()),
            AuditEventKind::PlaneInvoked {
                pack_id: ctx.pack_id().to_owned(),
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter: "conversation.safe_lane".to_owned(),
                delegated_core_adapter: None,
                operation: format!("conversation.safe_lane.{event_name}"),
                required_capabilities: Vec::new(),
            },
        );
    }
}

fn should_emit_safe_lane_event(
    config: &LoongClawConfig,
    event_name: &str,
    payload: &Value,
) -> bool {
    if !config.conversation.safe_lane_emit_runtime_events {
        return false;
    }

    if is_safe_lane_critical_event(event_name) {
        return true;
    }

    let sample_every = config.conversation.safe_lane_event_sample_every();
    if sample_every <= 1 {
        return true;
    }

    if config.conversation.safe_lane_event_adaptive_sampling
        && safe_lane_failure_pressure(payload)
            >= config
                .conversation
                .safe_lane_event_adaptive_failure_threshold() as u64
    {
        return true;
    }

    let round = payload
        .get("round")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    round.is_multiple_of(sample_every as u64)
}

fn is_safe_lane_critical_event(event_name: &str) -> bool {
    matches!(
        event_name,
        "lane_selected" | "verify_failed" | "final_status"
    )
}

fn safe_lane_failure_pressure(payload: &Value) -> u64 {
    let mut pressure = 0u64;

    if payload
        .get("status")
        .and_then(Value::as_str)
        .map(|status| status == "failed")
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("failure_kind")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("route_decision")
        .and_then(Value::as_str)
        .map(|decision| decision == "replan" || decision == "terminal")
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("failure_code")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("tool_output_stats")
        .and_then(|stats| stats.get("truncated_result_lines"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        > 0
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("metrics")
        .and_then(|metrics| metrics.get("verify_failures"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        > 0
    {
        pressure = pressure.saturating_add(1);
    }

    pressure
}

fn build_safe_lane_plan_graph(
    config: &LoongClawConfig,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    tool_node_max_attempts: u8,
    start_tool_index: usize,
) -> PlanGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let node_risk_tier = select_safe_lane_risk_tier(config, lane_decision);
    let normalized_start = start_tool_index.min(turn.tool_intents.len());
    for (index, intent) in turn.tool_intents.iter().enumerate().skip(normalized_start) {
        nodes.push(PlanNode {
            id: format!("tool-{}", index + 1),
            kind: PlanNodeKind::Tool,
            label: format!("invoke `{}`", intent.tool_name),
            tool_name: Some(intent.tool_name.clone()),
            timeout_ms: 3_000,
            max_attempts: tool_node_max_attempts,
            risk_tier: node_risk_tier,
        });
    }

    if config.conversation.safe_lane_verify_output_non_empty {
        nodes.push(PlanNode {
            id: "verify-1".to_owned(),
            kind: PlanNodeKind::Verify,
            label: "verify non-empty tool outputs".to_owned(),
            tool_name: None,
            timeout_ms: 500,
            max_attempts: 1,
            risk_tier: RiskTier::Medium,
        });
    }

    nodes.push(PlanNode {
        id: "respond-1".to_owned(),
        kind: PlanNodeKind::Respond,
        label: "compose final response".to_owned(),
        tool_name: None,
        timeout_ms: 500,
        max_attempts: 1,
        risk_tier: RiskTier::Low,
    });

    for pair in nodes.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        edges.push(PlanEdge {
            from: from.id.clone(),
            to: to.id.clone(),
        });
    }

    let max_total_attempts = nodes
        .iter()
        .map(|node| node.max_attempts as usize)
        .sum::<usize>()
        .max(1);
    PlanGraph {
        version: PLAN_GRAPH_VERSION.to_owned(),
        nodes,
        edges,
        budget: PlanBudget {
            max_nodes: 16,
            max_total_attempts,
            max_wall_time_ms: config.conversation.safe_lane_plan_max_wall_time_ms.max(1),
        },
    }
}

fn summarize_safe_lane_tool_output_stats(outputs: &[String]) -> SafeLaneToolOutputStats {
    let mut stats = SafeLaneToolOutputStats::default();
    for output in outputs {
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            stats.output_lines = stats.output_lines.saturating_add(1);
            if !line.starts_with('[') {
                continue;
            }
            stats.result_lines = stats.result_lines.saturating_add(1);
            if tool_result_contains_truncation_signal(line) {
                stats.truncated_result_lines = stats.truncated_result_lines.saturating_add(1);
            }
        }
    }
    stats
}

fn derive_safe_lane_runtime_health_signal(
    config: &LoongClawConfig,
    metrics: SafeLaneExecutionMetrics,
    final_status_failed: bool,
    final_failure_code: Option<&str>,
) -> SafeLaneRuntimeHealthSignal {
    let rounds_started = metrics.rounds_started as f64;
    let replan_rate = if rounds_started > 0.0 {
        metrics.replans_triggered as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        metrics.verify_failures as f64 / rounds_started
    } else {
        0.0
    };
    let aggregate_truncation_ratio = metrics
        .aggregate_tool_truncation_ratio_milli()
        .map(|milli| (milli as f64) / 1000.0);
    let truncation_warn_threshold = config
        .conversation
        .safe_lane_health_truncation_warn_threshold();
    let truncation_critical_threshold = config
        .conversation
        .safe_lane_health_truncation_critical_threshold();
    let verify_failure_warn_threshold = config
        .conversation
        .safe_lane_health_verify_failure_warn_threshold();
    let replan_warn_threshold = config.conversation.safe_lane_health_replan_warn_threshold();

    let mut flags = Vec::new();
    let mut has_critical = false;

    if let Some(ratio) = aggregate_truncation_ratio {
        if ratio >= truncation_critical_threshold {
            flags.push(format!("truncation_severe({ratio:.3})"));
            has_critical = true;
        } else if ratio >= truncation_warn_threshold {
            flags.push(format!("truncation_pressure({ratio:.3})"));
        }
    }

    if verify_failure_rate >= verify_failure_warn_threshold {
        flags.push(format!("verify_failure_pressure({verify_failure_rate:.3})"));
    }
    if replan_rate >= replan_warn_threshold {
        flags.push(format!("replan_pressure({replan_rate:.3})"));
    }

    let terminal_instability = final_status_failed
        && final_failure_code
            .map(|code| {
                code.contains("verify_failed")
                    || code.contains("backpressure")
                    || code.contains("session_governor")
            })
            .unwrap_or(false);
    if terminal_instability {
        flags.push("terminal_instability".to_owned());
        has_critical = true;
    }

    SafeLaneRuntimeHealthSignal {
        severity: if has_critical {
            "critical"
        } else if flags.is_empty() {
            "ok"
        } else {
            "warn"
        },
        flags,
    }
}

fn select_safe_lane_risk_tier(config: &LoongClawConfig, lane_decision: &LaneDecision) -> RiskTier {
    let high_risk_bar = config
        .conversation
        .safe_lane_risk_threshold
        .saturating_mul(2);
    let high_complexity_bar = config
        .conversation
        .safe_lane_complexity_threshold
        .saturating_mul(2);
    if lane_decision.risk_score >= high_risk_bar
        || lane_decision.complexity_score >= high_complexity_bar
    {
        RiskTier::High
    } else if lane_decision.risk_score > 0 || lane_decision.complexity_score > 0 {
        RiskTier::Medium
    } else {
        RiskTier::Low
    }
}

fn verify_safe_lane_final_output(
    config: &LoongClawConfig,
    output: &str,
    tool_intents: &[ToolIntent],
    adaptive_policy: SafeLaneAdaptiveVerifyPolicyState,
) -> PlanVerificationReport {
    let policy = PlanVerificationPolicy {
        require_non_empty: config.conversation.safe_lane_verify_output_non_empty,
        min_output_chars: config.conversation.safe_lane_verify_min_output_chars,
        require_status_prefix: config.conversation.safe_lane_verify_require_status_prefix,
        deny_markers: config
            .conversation
            .safe_lane_verify_deny_markers
            .iter()
            .map(|marker| marker.trim().to_ascii_lowercase())
            .filter(|marker| !marker.is_empty())
            .collect(),
    };
    let semantic_anchors = collect_semantic_anchors(tool_intents);
    let context = PlanVerificationContext {
        expected_result_lines: tool_intents.len().max(1),
        semantic_anchors,
        min_anchor_matches: adaptive_policy.min_anchor_matches,
    };
    verify_output(output, &context, &policy)
}

fn compute_safe_lane_verify_min_anchor_matches(
    config: &LoongClawConfig,
    verify_failures: u32,
) -> usize {
    if !config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation
    {
        return 0;
    }
    if verify_failures
        < config
            .conversation
            .safe_lane_verify_anchor_escalation_after_failures()
    {
        return 0;
    }
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches()
}

fn decide_safe_lane_session_governor(
    config: &LoongClawConfig,
    history: &SafeLaneGovernorHistorySignals,
) -> SafeLaneSessionGovernorDecision {
    let summary = &history.summary;
    let history_window_turns = config
        .conversation
        .safe_lane_session_governor_window_turns();
    let failed_final_status_events = summary
        .final_status_counts
        .get("failed")
        .copied()
        .unwrap_or_default();
    let backpressure_failure_events = count_safe_lane_backpressure_failures(summary);
    let failed_final_status_threshold = config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold();
    let backpressure_failure_threshold = config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold();
    let failed_threshold_triggered = failed_final_status_events >= failed_final_status_threshold;
    let backpressure_threshold_triggered =
        backpressure_failure_events >= backpressure_failure_threshold;
    let trend_enabled = config.conversation.safe_lane_session_governor_trend_enabled;
    let trend_samples = history.final_status_failed_samples.len();
    let trend_min_samples = config
        .conversation
        .safe_lane_session_governor_trend_min_samples();
    let trend_failure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold();
    let trend_backpressure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_trend_backpressure_ewma_threshold();
    let trend_ewma_alpha = config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha();
    let trend_ready = trend_enabled && trend_samples >= trend_min_samples;
    let trend_failure_ewma = if trend_ready {
        compute_ewma_bool(
            history.final_status_failed_samples.as_slice(),
            trend_ewma_alpha,
        )
    } else {
        None
    };
    let trend_backpressure_ewma = if trend_ready {
        compute_ewma_bool(
            history.backpressure_failure_samples.as_slice(),
            trend_ewma_alpha,
        )
    } else {
        None
    };
    let trend_threshold_triggered = trend_failure_ewma
        .map(|value| value >= trend_failure_ewma_threshold)
        .unwrap_or(false)
        || trend_backpressure_ewma
            .map(|value| value >= trend_backpressure_ewma_threshold)
            .unwrap_or(false);

    let recovery_success_streak = if trend_ready {
        trailing_success_streak(history.final_status_failed_samples.as_slice())
    } else {
        0
    };
    let recovery_success_streak_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_success_streak();
    let recovery_failure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_max_failure_ewma();
    let recovery_backpressure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_max_backpressure_ewma();
    let recovery_threshold_triggered = trend_ready
        && recovery_success_streak >= recovery_success_streak_threshold
        && trend_failure_ewma
            .map(|value| value <= recovery_failure_ewma_threshold)
            .unwrap_or(false)
        && trend_backpressure_ewma
            .map(|value| value <= recovery_backpressure_ewma_threshold)
            .unwrap_or(false);

    let engaged = config.conversation.safe_lane_session_governor_enabled
        && (failed_threshold_triggered
            || backpressure_threshold_triggered
            || trend_threshold_triggered)
        && !recovery_threshold_triggered;

    SafeLaneSessionGovernorDecision {
        engaged,
        history_window_turns,
        failed_final_status_events,
        failed_final_status_threshold,
        failed_threshold_triggered,
        backpressure_failure_events,
        backpressure_failure_threshold,
        backpressure_threshold_triggered,
        trend_enabled,
        trend_samples,
        trend_min_samples,
        trend_failure_ewma,
        trend_failure_ewma_threshold,
        trend_backpressure_ewma,
        trend_backpressure_ewma_threshold,
        trend_threshold_triggered,
        recovery_success_streak,
        recovery_success_streak_threshold,
        recovery_failure_ewma_threshold,
        recovery_backpressure_ewma_threshold,
        recovery_threshold_triggered,
        force_no_replan: engaged
            && config
                .conversation
                .safe_lane_session_governor_force_no_replan,
        forced_node_max_attempts: engaged.then(|| {
            config
                .conversation
                .safe_lane_session_governor_force_node_max_attempts()
        }),
    }
}

async fn load_safe_lane_history_signals_for_governor(
    config: &LoongClawConfig,
    session_id: &str,
    kernel_ctx: Option<&KernelContext>,
) -> SafeLaneGovernorHistorySignals {
    if !config.conversation.safe_lane_session_governor_enabled {
        return SafeLaneGovernorHistorySignals::default();
    }

    let window_turns = config
        .conversation
        .safe_lane_session_governor_window_turns();
    if let Some(ctx) = kernel_ctx {
        let request = MemoryCoreRequest {
            operation: crate::memory::MEMORY_OP_WINDOW.to_owned(),
            payload: json!({
                "session_id": session_id,
                "limit": window_turns,
                "allow_extended_limit": true,
            }),
        };
        let caps = BTreeSet::from([Capability::MemoryRead]);
        if let Ok(outcome) = ctx
            .kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await
        {
            let assistant_contents =
                collect_assistant_contents_from_memory_window_payload(outcome.payload.get("turns"));
            return summarize_governor_history_signals(
                assistant_contents.iter().map(String::as_str),
            );
        }
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if let Ok(turns) = crate::memory::window_direct_extended(session_id, window_turns) {
            let assistant_contents = turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str()))
                .collect::<Vec<_>>();
            return summarize_governor_history_signals(assistant_contents);
        }
    }

    SafeLaneGovernorHistorySignals::default()
}

fn summarize_governor_history_signals<'a, I>(
    assistant_contents: I,
) -> SafeLaneGovernorHistorySignals
where
    I: IntoIterator<Item = &'a str>,
{
    let mut retained_contents = Vec::new();
    let mut final_status_failed_samples = Vec::new();
    let mut backpressure_failure_samples = Vec::new();

    for content in assistant_contents {
        retained_contents.push(content.to_owned());
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        if record.event != "final_status" {
            continue;
        }
        match record.payload.get("status").and_then(Value::as_str) {
            Some("failed") => {
                final_status_failed_samples.push(true);
                backpressure_failure_samples
                    .push(is_backpressure_final_status_payload(&record.payload));
            }
            Some("succeeded") => {
                final_status_failed_samples.push(false);
                backpressure_failure_samples.push(false);
            }
            _ => {}
        }
    }

    SafeLaneGovernorHistorySignals {
        summary: summarize_safe_lane_events(retained_contents.iter().map(String::as_str)),
        final_status_failed_samples,
        backpressure_failure_samples,
    }
}

fn collect_assistant_contents_from_memory_window_payload(
    turns_payload: Option<&Value>,
) -> Vec<String> {
    turns_payload
        .and_then(Value::as_array)
        .map(|turns| {
            turns
                .iter()
                .filter_map(|turn| {
                    (turn.get("role").and_then(Value::as_str) == Some("assistant"))
                        .then_some(turn.get("content").and_then(Value::as_str))
                        .flatten()
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn count_safe_lane_backpressure_failures(summary: &SafeLaneEventSummary) -> u32 {
    summary
        .failure_code_counts
        .get("safe_lane_plan_backpressure_guard")
        .copied()
        .unwrap_or_default()
        .saturating_add(
            summary
                .failure_code_counts
                .get("safe_lane_plan_verify_failed_backpressure_guard")
                .copied()
                .unwrap_or_default(),
        )
}

fn is_backpressure_final_status_payload(payload: &Value) -> bool {
    if payload
        .get("failure_code")
        .and_then(Value::as_str)
        .map(|code| {
            matches!(
                code,
                "safe_lane_plan_backpressure_guard"
                    | "safe_lane_plan_verify_failed_backpressure_guard"
            )
        })
        .unwrap_or(false)
    {
        return true;
    }
    payload
        .get("route_reason")
        .and_then(Value::as_str)
        .map(|reason| reason.starts_with("backpressure_"))
        .unwrap_or(false)
}

fn compute_ewma_bool(samples: &[bool], alpha: f64) -> Option<f64> {
    let mut iter = samples.iter();
    let first = iter.next().copied()?;
    let mut ewma = if first { 1.0 } else { 0.0 };
    for sample in iter {
        let value = if *sample { 1.0 } else { 0.0 };
        ewma = (alpha * value) + ((1.0 - alpha) * ewma);
    }
    Some(ewma)
}

fn trailing_success_streak(failed_samples: &[bool]) -> u32 {
    let mut streak = 0u32;
    for failed in failed_samples.iter().rev() {
        if *failed {
            break;
        }
        streak = streak.saturating_add(1);
    }
    streak
}

fn apply_safe_lane_session_governor_route_override(
    route: SafeLaneFailureRoute,
    governor: SafeLaneSessionGovernorDecision,
) -> SafeLaneFailureRoute {
    if governor.force_no_replan && route.reason == "round_budget_exhausted" {
        return SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "session_governor_no_replan",
        };
    }
    route
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafeLaneFailureRouteDecision {
    Replan,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SafeLaneFailureRoute {
    decision: SafeLaneFailureRouteDecision,
    reason: &'static str,
}

fn route_safe_lane_failure(
    failure: &TurnFailure,
    round: u8,
    max_rounds: u8,
) -> SafeLaneFailureRoute {
    if round >= max_rounds {
        return SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "round_budget_exhausted",
        };
    }

    match failure.code.as_str() {
        "safe_lane_plan_node_policy_denied"
        | "kernel_policy_denied"
        | "tool_not_found"
        | "max_tool_steps_exceeded"
        | "no_kernel_context" => {
            return SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: "policy_denied",
            };
        }
        "safe_lane_plan_verify_failed" => {
            if failure.retryable {
                return SafeLaneFailureRoute {
                    decision: SafeLaneFailureRouteDecision::Replan,
                    reason: "retryable_failure",
                };
            }
            return SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: "non_retryable_failure",
            };
        }
        "safe_lane_plan_node_retryable_error" | "tool_execution_failed" => {
            if failure.retryable {
                return SafeLaneFailureRoute {
                    decision: SafeLaneFailureRouteDecision::Replan,
                    reason: "retryable_failure",
                };
            }
            return SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: "retryable_flag_false",
            };
        }
        "safe_lane_plan_verify_failed_budget_exhausted" => {
            return SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: "round_budget_exhausted",
            };
        }
        "safe_lane_plan_validation_failed"
        | "safe_lane_plan_topology_resolution_failed"
        | "safe_lane_plan_budget_exceeded"
        | "safe_lane_plan_wall_time_exceeded"
        | "safe_lane_plan_node_non_retryable_error"
        | "kernel_execution_failed" => {
            return SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: "non_retryable_failure",
            };
        }
        _ => {}
    }

    match failure.kind {
        TurnFailureKind::Retryable if failure.retryable => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: "retryable_failure",
        },
        TurnFailureKind::Retryable => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "retryable_flag_false",
        },
        TurnFailureKind::PolicyDenied => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "policy_denied",
        },
        TurnFailureKind::NonRetryable => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "non_retryable_failure",
        },
        TurnFailureKind::ApprovalRequired => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "approval_required",
        },
        TurnFailureKind::Provider => SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "provider_failure",
        },
    }
}

fn apply_safe_lane_backpressure_guard(
    config: &LoongClawConfig,
    route: SafeLaneFailureRoute,
    metrics: SafeLaneExecutionMetrics,
) -> SafeLaneFailureRoute {
    if !config.conversation.safe_lane_backpressure_guard_enabled
        || !matches!(route.decision, SafeLaneFailureRouteDecision::Replan)
    {
        return route;
    }

    if metrics.total_attempts_used
        >= config
            .conversation
            .safe_lane_backpressure_max_total_attempts()
    {
        return SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "backpressure_attempts_exhausted",
        };
    }

    if metrics.replans_triggered >= config.conversation.safe_lane_backpressure_max_replans() {
        return SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "backpressure_replans_exhausted",
        };
    }

    route
}

fn format_safe_lane_route_decision(decision: SafeLaneFailureRouteDecision) -> &'static str {
    match decision {
        SafeLaneFailureRouteDecision::Replan => "replan",
        SafeLaneFailureRouteDecision::Terminal => "terminal",
    }
}

fn summarize_verify_terminal_reason(route_reason: &str) -> &'static str {
    match route_reason {
        "round_budget_exhausted" => "verify_failed_budget_exhausted",
        "backpressure_attempts_exhausted" | "backpressure_replans_exhausted" => {
            "verify_failed_backpressure_guard"
        }
        "session_governor_no_replan" => "verify_failed_session_governor",
        _ => "verify_failed_non_retryable",
    }
}

fn should_replan_for_verification_failure(report: &PlanVerificationReport) -> bool {
    !report.failure_codes.iter().any(|code| {
        matches!(
            code,
            PlanVerificationFailureCode::DenyMarkerDetected
                | PlanVerificationFailureCode::MissingStatusPrefix
                | PlanVerificationFailureCode::MissingSemanticAnchors
        )
    })
}

fn format_verification_failure_code(code: &PlanVerificationFailureCode) -> &'static str {
    match code {
        PlanVerificationFailureCode::EmptyOutput => "empty_output",
        PlanVerificationFailureCode::OutputTooShort => "output_too_short",
        PlanVerificationFailureCode::DenyMarkerDetected => "deny_marker_detected",
        PlanVerificationFailureCode::InsufficientResultLines => "insufficient_result_lines",
        PlanVerificationFailureCode::MissingStatusPrefix => "missing_status_prefix",
        PlanVerificationFailureCode::FailureStatusDetected => "failure_status_detected",
        PlanVerificationFailureCode::MissingSemanticAnchors => "missing_semantic_anchors",
    }
}

fn collect_semantic_anchors(tool_intents: &[ToolIntent]) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    for intent in tool_intents {
        collect_value_anchors(None, &intent.args_json, &mut anchors);
    }
    anchors
}

fn collect_value_anchors(parent_key: Option<&str>, value: &Value, anchors: &mut BTreeSet<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match value {
        Value::String(text) => {
            if parent_key.map(is_anchor_key_allowed).unwrap_or(false) {
                push_anchor_candidate(text.as_str(), anchors);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_value_anchors(parent_key, item, anchors);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if is_sensitive_key(key.as_str()) {
                    continue;
                }
                collect_value_anchors(Some(key.as_str()), item, anchors);
            }
        }
        _ => {}
    }
}

fn is_anchor_key_allowed(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "path"
            | "file"
            | "filename"
            | "url"
            | "endpoint"
            | "target"
            | "query"
            | "operation"
            | "command"
            | "cwd"
            | "dir"
            | "directory"
    )
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "credential",
        "api_key",
        "apikey",
        "auth",
        "authorization",
        "cookie",
        "session",
        "bearer",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn push_anchor_candidate(text: &str, anchors: &mut BTreeSet<String>) {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.len() < 3 || normalized.len() > 96 {
        return;
    }
    if normalized.contains(' ') {
        return;
    }
    anchors.insert(normalized.clone());
    if let Some(last_segment) = normalized.rsplit('/').next()
        && last_segment.len() >= 3
    {
        anchors.insert(last_segment.to_owned());
    }
}

async fn derive_replan_cursor(
    failure: &PlanRunFailure,
    executor: &SafeLanePlanNodeExecutor<'_>,
    tool_count: usize,
) -> (usize, Vec<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match failure {
        PlanRunFailure::NodeFailed { node_id, .. } => {
            if let Ok(index) = parse_tool_node_index(node_id.as_str())
                && index < tool_count
            {
                return (index, executor.tool_outputs_snapshot().await);
            }
            (0, Vec::new())
        }
        _ => (0, Vec::new()),
    }
}

fn summarize_plan_failure(failure: &PlanRunFailure) -> String {
    match failure {
        PlanRunFailure::ValidationFailed(error) => {
            format!("validation_failed:{error}")
        }
        PlanRunFailure::TopologyResolutionFailed => "topology_resolution_failed".to_owned(),
        PlanRunFailure::BudgetExceeded {
            attempts_used,
            limit,
        } => {
            format!("budget_exceeded attempts_used={attempts_used} limit={limit}")
        }
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms,
            limit_ms,
        } => {
            format!("wall_time_exceeded elapsed_ms={elapsed_ms} limit_ms={limit_ms}")
        }
        PlanRunFailure::NodeFailed {
            node_id,
            last_error_kind,
            last_error,
            ..
        } => {
            format!("node_failed node={node_id} error_kind={last_error_kind:?} reason={last_error}")
        }
    }
}

fn format_turn_failure_kind(kind: TurnFailureKind) -> &'static str {
    match kind {
        TurnFailureKind::ApprovalRequired => "approval_required",
        TurnFailureKind::PolicyDenied => "policy_denied",
        TurnFailureKind::Retryable => "retryable",
        TurnFailureKind::NonRetryable => "non_retryable",
        TurnFailureKind::Provider => "provider",
    }
}

fn turn_failure_from_plan_failure(failure: &PlanRunFailure) -> TurnFailure {
    match failure {
        PlanRunFailure::ValidationFailed(error) => TurnFailure::non_retryable(
            "safe_lane_plan_validation_failed",
            format!("safe_lane_plan_validation_failed: {error}"),
        ),
        PlanRunFailure::TopologyResolutionFailed => TurnFailure::non_retryable(
            "safe_lane_plan_topology_resolution_failed",
            "safe_lane_plan_topology_resolution_failed",
        ),
        PlanRunFailure::BudgetExceeded {
            attempts_used,
            limit,
        } => TurnFailure::non_retryable(
            "safe_lane_plan_budget_exceeded",
            format!("safe_lane_plan_budget_exceeded attempts_used={attempts_used} limit={limit}"),
        ),
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms,
            limit_ms,
        } => TurnFailure::non_retryable(
            "safe_lane_plan_wall_time_exceeded",
            format!(
                "safe_lane_plan_wall_time_exceeded elapsed_ms={elapsed_ms} limit_ms={limit_ms}"
            ),
        ),
        PlanRunFailure::NodeFailed {
            last_error,
            last_error_kind,
            ..
        } => match last_error_kind {
            PlanNodeErrorKind::PolicyDenied => {
                TurnFailure::policy_denied("safe_lane_plan_node_policy_denied", last_error.clone())
            }
            PlanNodeErrorKind::Retryable => {
                TurnFailure::retryable("safe_lane_plan_node_retryable_error", last_error.clone())
            }
            PlanNodeErrorKind::NonRetryable => TurnFailure::non_retryable(
                "safe_lane_plan_node_non_retryable_error",
                last_error.clone(),
            ),
        },
    }
}

fn turn_failure_from_verify_failure(verify_error: &str, retryable: bool) -> TurnFailure {
    let reason = format!("safe_lane_plan_verify_failed: {verify_error}");
    if retryable {
        TurnFailure::retryable("safe_lane_plan_verify_failed", reason)
    } else {
        TurnFailure::non_retryable("safe_lane_plan_verify_failed", reason)
    }
}

fn terminal_turn_failure_from_verify_failure(
    verify_error: &str,
    retryable_signal: bool,
    route_reason: &str,
) -> TurnFailure {
    let reason = format!("safe_lane_plan_verify_failed: {verify_error}");
    match route_reason {
        "round_budget_exhausted" if retryable_signal => {
            // Retryable at signal layer, but terminal after exhausting rounds.
            TurnFailure::non_retryable("safe_lane_plan_verify_failed_budget_exhausted", reason)
        }
        "backpressure_attempts_exhausted" | "backpressure_replans_exhausted" => {
            TurnFailure::non_retryable("safe_lane_plan_verify_failed_backpressure_guard", reason)
        }
        "session_governor_no_replan" => {
            TurnFailure::non_retryable("safe_lane_plan_verify_failed_session_governor", reason)
        }
        _ => TurnFailure::non_retryable("safe_lane_plan_verify_failed", reason),
    }
}

fn turn_result_from_plan_failure(failure: PlanRunFailure) -> TurnResult {
    let failure_meta = turn_failure_from_plan_failure(&failure);
    if matches!(failure_meta.kind, TurnFailureKind::PolicyDenied) {
        TurnResult::ToolDenied(failure_meta)
    } else {
        TurnResult::ToolError(failure_meta)
    }
}

fn terminal_turn_result_from_plan_failure_with_route(
    failure: PlanRunFailure,
    route: SafeLaneFailureRoute,
) -> TurnResult {
    if matches!(
        route.reason,
        "backpressure_attempts_exhausted" | "backpressure_replans_exhausted"
    ) {
        let summary = summarize_plan_failure(&failure);
        return TurnResult::ToolError(TurnFailure::non_retryable(
            "safe_lane_plan_backpressure_guard",
            format!("safe_lane_plan_backpressure_guard: {summary}"),
        ));
    }
    if route.reason == "session_governor_no_replan" {
        let summary = summarize_plan_failure(&failure);
        return TurnResult::ToolError(TurnFailure::non_retryable(
            "safe_lane_plan_session_governor_no_replan",
            format!("safe_lane_plan_session_governor_no_replan: {summary}"),
        ));
    }
    turn_result_from_plan_failure(failure)
}

struct SafeLanePlanNodeExecutor<'a> {
    tool_intents: &'a [ToolIntent],
    kernel_ctx: Option<&'a KernelContext>,
    verify_output_non_empty: bool,
    tool_outputs: Mutex<Vec<String>>,
    tool_result_payload_summary_limit_chars: usize,
}

impl<'a> SafeLanePlanNodeExecutor<'a> {
    fn new(
        tool_intents: &'a [ToolIntent],
        kernel_ctx: Option<&'a KernelContext>,
        verify_output_non_empty: bool,
        seed_tool_outputs: Vec<String>,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            tool_intents,
            kernel_ctx,
            verify_output_non_empty,
            tool_outputs: Mutex::new(seed_tool_outputs),
            tool_result_payload_summary_limit_chars,
        }
    }

    async fn tool_outputs_snapshot(&self) -> Vec<String> {
        self.tool_outputs.lock().await.clone()
    }
}

#[async_trait]
impl PlanNodeExecutor for SafeLanePlanNodeExecutor<'_> {
    async fn execute(&self, node: &PlanNode, _attempt: u8) -> Result<(), PlanNodeError> {
        match node.kind {
            PlanNodeKind::Tool => {
                let index = parse_tool_node_index(node.id.as_str())?;
                let intent = self.tool_intents.get(index).ok_or_else(|| {
                    PlanNodeError::non_retryable(format!(
                        "missing tool intent for node `{}`",
                        node.id
                    ))
                })?;
                let output = execute_single_tool_intent(
                    intent,
                    self.kernel_ctx,
                    self.tool_result_payload_summary_limit_chars,
                )
                .await?;
                self.tool_outputs.lock().await.push(output);
                Ok(())
            }
            PlanNodeKind::Verify => {
                if !self.verify_output_non_empty {
                    return Ok(());
                }
                let outputs = self.tool_outputs.lock().await;
                if outputs.is_empty() || outputs.iter().any(|line| line.trim().is_empty()) {
                    return Err(PlanNodeError::non_retryable(
                        "verify_failed:empty_tool_output".to_owned(),
                    ));
                }
                Ok(())
            }
            PlanNodeKind::Transform | PlanNodeKind::Respond => Ok(()),
        }
    }
}

fn parse_tool_node_index(node_id: &str) -> Result<usize, PlanNodeError> {
    let suffix = node_id
        .strip_prefix("tool-")
        .ok_or_else(|| PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`")))?;
    let parsed = suffix.parse::<usize>().map_err(|error| {
        PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`: {error}"))
    })?;
    if parsed == 0 {
        return Err(PlanNodeError::non_retryable(format!(
            "invalid tool node ordinal in `{node_id}`"
        )));
    }
    Ok(parsed - 1)
}

async fn execute_single_tool_intent(
    intent: &ToolIntent,
    kernel_ctx: Option<&KernelContext>,
    payload_summary_limit_chars: usize,
) -> Result<String, PlanNodeError> {
    if !crate::tools::is_known_tool_name(&intent.tool_name) {
        return Err(PlanNodeError::policy_denied(format!(
            "tool_not_found: {}",
            intent.tool_name
        )));
    }
    let ctx =
        kernel_ctx.ok_or_else(|| PlanNodeError::policy_denied("no_kernel_context".to_owned()))?;
    let request = ToolCoreRequest {
        tool_name: intent.tool_name.clone(),
        payload: intent.args_json.clone(),
    };
    let caps = BTreeSet::from([Capability::InvokeTool]);
    let outcome = ctx
        .kernel
        .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
        .await
        .map_err(|error| {
            let kind = match classify_kernel_error(&error) {
                KernelFailureClass::PolicyDenied => PlanNodeErrorKind::PolicyDenied,
                KernelFailureClass::RetryableExecution => PlanNodeErrorKind::Retryable,
                KernelFailureClass::NonRetryable => PlanNodeErrorKind::NonRetryable,
            };
            PlanNodeError {
                kind,
                message: format!("{error}"),
            }
        })?;
    Ok(super::turn_engine::format_tool_result_line_with_limit(
        intent,
        &outcome,
        payload_summary_limit_chars,
    ))
}

fn build_tool_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    if let Some(skill_context) = parse_external_skill_invoke_context(tool_result_text) {
        messages.push(json!({
            "role": "system",
            "content": build_external_skill_system_message(&skill_context),
        }));
        messages.push(json!({
            "role": "user",
            "content": build_external_skill_followup_user_prompt(user_input, &skill_context),
        }));
        return messages;
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_result]\n{tool_result_text}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_user_prompt(user_input, None, Some(tool_result_text)),
    }));
    messages
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalSkillInvokeContext {
    skill_id: String,
    display_name: String,
    instructions: String,
}

fn parse_external_skill_invoke_context(
    tool_result_text: &str,
) -> Option<ExternalSkillInvokeContext> {
    let trimmed = tool_result_text.trim();
    let mut lines = trimmed.lines().filter(|line| !line.trim().is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let payload = line.strip_prefix("[ok] ")?;
    let envelope: Value = serde_json::from_str(payload).ok()?;
    if envelope.get("tool")?.as_str()? != "external_skills.invoke" {
        return None;
    }
    let payload_summary = envelope.get("payload_summary")?.as_str()?;
    let payload_json: Value = serde_json::from_str(payload_summary).ok()?;
    let instructions = payload_json
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let skill_id = payload_json
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("external-skill")
        .to_owned();
    let display_name = payload_json
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(skill_id.as_str())
        .to_owned();
    Some(ExternalSkillInvokeContext {
        skill_id,
        display_name,
        instructions,
    })
}

fn build_external_skill_system_message(skill_context: &ExternalSkillInvokeContext) -> String {
    format!(
        "Managed external skill `{}` ({}) is now active for this task. Treat the following `SKILL.md` content as trusted runtime guidance until superseded.\n\n{}",
        skill_context.skill_id, skill_context.display_name, skill_context.instructions
    )
}

fn build_external_skill_followup_user_prompt(
    user_input: &str,
    skill_context: &ExternalSkillInvokeContext,
) -> String {
    [
        EXTERNAL_SKILL_FOLLOWUP_PROMPT.to_owned(),
        format!(
            "Loaded managed external skill:\n- id: {}\n- name: {}",
            skill_context.skill_id, skill_context.display_name
        ),
        format!("Original request:\n{user_input}"),
    ]
    .join("\n\n")
}

fn build_tool_failure_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_failure]\n{tool_failure_reason}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_user_prompt(user_input, None, None),
    }));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tool_followup_messages_include_truncation_hint_for_truncated_tool_results() {
        let messages = build_tool_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#,
            "summarize note.md",
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(
            user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT)
        );
        assert!(user_prompt.contains("Original request:\nsummarize note.md"));
    }

    #[test]
    fn build_tool_failure_followup_messages_do_not_include_truncation_hint() {
        let messages = build_tool_failure_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            "tool_timeout ...(truncated 200 chars)",
            "summarize note.md",
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
    fn build_tool_followup_messages_promotes_external_skill_invoke_to_system_context() {
        let messages = build_tool_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#,
            "summarize note.md",
        );

        assert!(
            messages.iter().any(|message| message.get("role")
                == Some(&Value::String("system".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content
                        .contains("Follow the managed skill instruction before answering."))
                    .unwrap_or(false)),
            "safe-lane followup should promote invoked external skill instructions into system context: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .filter(
                    |message| message.get("role") == Some(&Value::String("assistant".to_owned()))
                )
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .all(|content| !content.contains("[tool_result]\n[ok]")),
            "safe-lane followup should not carry invoke payload forward as an ordinary assistant tool_result: {messages:?}"
        );
    }

    #[test]
    fn safe_lane_route_retryable_failure_replans_with_remaining_budget() {
        let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
        let route = route_safe_lane_failure(&failure, 0, 1);

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Replan);
        assert_eq!(route.reason, "retryable_failure");
    }

    #[test]
    fn safe_lane_route_retryable_failure_becomes_terminal_after_budget_exhaustion() {
        let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
        let route = route_safe_lane_failure(&failure, 1, 1);

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(route.reason, "round_budget_exhausted");
    }

    #[test]
    fn safe_lane_route_policy_denied_failure_is_terminal() {
        let failure = TurnFailure::policy_denied("safe_lane_plan_node_policy_denied", "denied");
        let route = route_safe_lane_failure(&failure, 0, 3);

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(route.reason, "policy_denied");
    }

    #[test]
    fn safe_lane_route_non_retryable_failure_is_terminal() {
        let failure = TurnFailure::non_retryable("safe_lane_plan_node_non_retryable_error", "bad");
        let route = route_safe_lane_failure(&failure, 0, 3);

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(route.reason, "non_retryable_failure");
    }

    #[test]
    fn turn_failure_from_plan_failure_node_error_mapping_is_stable() {
        let cases = [
            (
                PlanNodeErrorKind::PolicyDenied,
                TurnFailureKind::PolicyDenied,
                "safe_lane_plan_node_policy_denied",
                false,
            ),
            (
                PlanNodeErrorKind::Retryable,
                TurnFailureKind::Retryable,
                "safe_lane_plan_node_retryable_error",
                true,
            ),
            (
                PlanNodeErrorKind::NonRetryable,
                TurnFailureKind::NonRetryable,
                "safe_lane_plan_node_non_retryable_error",
                false,
            ),
        ];

        for (node_kind, expected_kind, expected_code, expected_retryable) in cases {
            let failure = PlanRunFailure::NodeFailed {
                node_id: "tool-1".to_owned(),
                attempts_used: 1,
                last_error_kind: node_kind,
                last_error: "boom".to_owned(),
            };
            let mapped = turn_failure_from_plan_failure(&failure);
            assert_eq!(mapped.kind, expected_kind, "node_kind={node_kind:?}");
            assert_eq!(mapped.code, expected_code, "node_kind={node_kind:?}");
            assert_eq!(
                mapped.retryable, expected_retryable,
                "node_kind={node_kind:?}"
            );
        }
    }

    #[test]
    fn turn_failure_from_plan_failure_static_failure_mapping_is_stable() {
        let failures = [
            PlanRunFailure::ValidationFailed("invalid".to_owned()),
            PlanRunFailure::TopologyResolutionFailed,
            PlanRunFailure::BudgetExceeded {
                attempts_used: 5,
                limit: 4,
            },
            PlanRunFailure::WallTimeExceeded {
                elapsed_ms: 1200,
                limit_ms: 1000,
            },
        ];

        for failure in failures {
            let mapped = turn_failure_from_plan_failure(&failure);
            assert_eq!(mapped.kind, TurnFailureKind::NonRetryable);
            assert!(!mapped.retryable);
            assert!(
                mapped.code.starts_with("safe_lane_plan_"),
                "unexpected code: {}",
                mapped.code
            );
        }
    }

    #[test]
    fn safe_lane_event_sampling_keeps_critical_events() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_emit_runtime_events = true;
        config.conversation.safe_lane_event_sample_every = 3;

        let emitted = should_emit_safe_lane_event(
            &config,
            "final_status",
            &json!({
                "round": 1
            }),
        );
        assert!(emitted, "critical final_status event must always emit");
    }

    #[test]
    fn safe_lane_event_sampling_skips_non_critical_rounds() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_emit_runtime_events = true;
        config.conversation.safe_lane_event_sample_every = 2;
        config.conversation.safe_lane_event_adaptive_sampling = false;

        let emit_round_0 = should_emit_safe_lane_event(
            &config,
            "plan_round_started",
            &json!({
                "round": 0
            }),
        );
        let emit_round_1 = should_emit_safe_lane_event(
            &config,
            "plan_round_started",
            &json!({
                "round": 1
            }),
        );

        assert!(emit_round_0, "round 0 should pass sampling gate");
        assert!(!emit_round_1, "round 1 should be sampled out");
    }

    #[test]
    fn safe_lane_event_sampling_adaptive_mode_keeps_failure_pressure_events() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_emit_runtime_events = true;
        config.conversation.safe_lane_event_sample_every = 4;
        config.conversation.safe_lane_event_adaptive_sampling = true;
        config
            .conversation
            .safe_lane_event_adaptive_failure_threshold = 1;

        let emitted = should_emit_safe_lane_event(
            &config,
            "plan_round_completed",
            &json!({
                "round": 1,
                "failure_code": "safe_lane_plan_node_retryable_error",
                "route_decision": "replan",
                "metrics": {
                    "rounds_started": 2,
                    "rounds_succeeded": 0,
                    "rounds_failed": 1,
                    "verify_failures": 0,
                    "replans_triggered": 1,
                    "total_attempts_used": 2
                }
            }),
        );

        assert!(
            emitted,
            "adaptive failure-pressure sampling should force emit for troubleshooting"
        );
    }

    #[test]
    fn safe_lane_event_sampling_adaptive_mode_can_be_disabled() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_emit_runtime_events = true;
        config.conversation.safe_lane_event_sample_every = 4;
        config.conversation.safe_lane_event_adaptive_sampling = false;
        config
            .conversation
            .safe_lane_event_adaptive_failure_threshold = 1;

        let emitted = should_emit_safe_lane_event(
            &config,
            "plan_round_completed",
            &json!({
                "round": 1,
                "failure_code": "safe_lane_plan_node_retryable_error",
                "route_decision": "replan",
                "metrics": {
                    "rounds_started": 2,
                    "rounds_succeeded": 0,
                    "rounds_failed": 1,
                    "verify_failures": 0,
                    "replans_triggered": 1,
                    "total_attempts_used": 2
                }
            }),
        );

        assert!(
            !emitted,
            "with adaptive sampling disabled, round-based sampling should still drop this event"
        );
    }

    #[test]
    fn safe_lane_failure_pressure_counts_truncated_tool_output_stats() {
        let payload = json!({
            "tool_output_stats": {
                "output_lines": 1,
                "result_lines": 1,
                "truncated_result_lines": 1,
                "any_truncated": true,
                "truncation_ratio_milli": 1000
            }
        });
        assert_eq!(safe_lane_failure_pressure(&payload), 1);
    }

    #[test]
    fn safe_lane_tool_output_stats_detect_truncated_result_lines() {
        let outputs = vec![
            "[ok] {\"payload_truncated\":true}".to_owned(),
            "[ok] {\"payload_truncated\":false}\n[tool_result_truncated] removed_chars=2"
                .to_owned(),
            "plain diagnostic line".to_owned(),
        ];

        let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
        assert_eq!(stats.output_lines, 4);
        assert_eq!(stats.result_lines, 3);
        assert_eq!(stats.truncated_result_lines, 2);
        assert_eq!(stats.truncation_ratio_milli(), 666);
        let encoded = stats.as_json();
        assert_eq!(encoded["any_truncated"], true);
        assert_eq!(encoded["truncation_ratio_milli"], 666);
    }

    #[test]
    fn safe_lane_tool_output_stats_handles_mixed_multiline_blocks() {
        let outputs = vec![
            "\n[ok] {\"payload_truncated\":false}\nnot a result line\n[ok] {\"payload_truncated\":true}\n"
                .to_owned(),
            "[result] completed\n\n[ok] {\"payload_truncated\":false}".to_owned(),
        ];

        let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
        assert_eq!(stats.output_lines, 5);
        assert_eq!(stats.result_lines, 4);
        assert_eq!(stats.truncated_result_lines, 1);
        assert_eq!(stats.truncation_ratio_milli(), 250);
        let encoded = stats.as_json();
        assert_eq!(encoded["any_truncated"], true);
        assert_eq!(encoded["truncation_ratio_milli"], 250);
    }

    #[test]
    fn runtime_health_signal_marks_warn_on_truncation_pressure() {
        let mut config = LoongClawConfig::default();
        config
            .conversation
            .safe_lane_health_truncation_warn_threshold = 0.20;
        config
            .conversation
            .safe_lane_health_truncation_critical_threshold = 0.50;
        let metrics = SafeLaneExecutionMetrics {
            rounds_started: 2,
            tool_output_result_lines_total: 4,
            tool_output_truncated_result_lines_total: 1,
            ..SafeLaneExecutionMetrics::default()
        };

        let signal = derive_safe_lane_runtime_health_signal(&config, metrics, false, None);
        assert_eq!(signal.severity, "warn");
        assert!(
            signal
                .flags
                .iter()
                .any(|value| value.contains("truncation_pressure(0.250)"))
        );
    }

    #[test]
    fn runtime_health_signal_marks_critical_on_terminal_instability() {
        let config = LoongClawConfig::default();
        let metrics = SafeLaneExecutionMetrics {
            rounds_started: 2,
            verify_failures: 1,
            replans_triggered: 1,
            tool_output_result_lines_total: 2,
            tool_output_truncated_result_lines_total: 1,
            ..SafeLaneExecutionMetrics::default()
        };

        let signal = derive_safe_lane_runtime_health_signal(
            &config,
            metrics,
            true,
            Some("safe_lane_plan_verify_failed_session_governor"),
        );
        assert_eq!(signal.severity, "critical");
        assert!(
            signal
                .flags
                .iter()
                .any(|value| value == "terminal_instability")
        );
    }

    #[test]
    fn verify_anchor_policy_escalates_after_configured_failures() {
        let mut config = LoongClawConfig::default();
        config
            .conversation
            .safe_lane_verify_adaptive_anchor_escalation = true;
        config
            .conversation
            .safe_lane_verify_anchor_escalation_after_failures = 2;
        config
            .conversation
            .safe_lane_verify_anchor_escalation_min_matches = 1;

        assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 0), 0);
        assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 1), 0);
        assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 2), 1);
        assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 1);
    }

    #[test]
    fn verify_anchor_policy_escalation_can_be_disabled() {
        let mut config = LoongClawConfig::default();
        config
            .conversation
            .safe_lane_verify_adaptive_anchor_escalation = false;
        config
            .conversation
            .safe_lane_verify_anchor_escalation_after_failures = 1;
        config
            .conversation
            .safe_lane_verify_anchor_escalation_min_matches = 3;

        assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 0);
    }

    #[test]
    fn backpressure_guard_blocks_replan_when_attempt_budget_exhausted() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_backpressure_guard_enabled = true;
        config
            .conversation
            .safe_lane_backpressure_max_total_attempts = 2;
        config.conversation.safe_lane_backpressure_max_replans = 10;

        let route = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: "retryable_failure",
        };
        let metrics = SafeLaneExecutionMetrics {
            total_attempts_used: 2,
            ..SafeLaneExecutionMetrics::default()
        };
        let guarded = apply_safe_lane_backpressure_guard(&config, route, metrics);
        assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(guarded.reason, "backpressure_attempts_exhausted");
    }

    #[test]
    fn backpressure_guard_blocks_replan_when_replan_budget_exhausted() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_backpressure_guard_enabled = true;
        config
            .conversation
            .safe_lane_backpressure_max_total_attempts = 10;
        config.conversation.safe_lane_backpressure_max_replans = 1;

        let route = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: "retryable_failure",
        };
        let metrics = SafeLaneExecutionMetrics {
            replans_triggered: 1,
            ..SafeLaneExecutionMetrics::default()
        };
        let guarded = apply_safe_lane_backpressure_guard(&config, route, metrics);
        assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(guarded.reason, "backpressure_replans_exhausted");
    }

    fn governor_history_with_summary(
        summary: SafeLaneEventSummary,
    ) -> SafeLaneGovernorHistorySignals {
        SafeLaneGovernorHistorySignals {
            summary,
            ..SafeLaneGovernorHistorySignals::default()
        }
    }

    #[test]
    fn summarize_governor_history_signals_extracts_failure_samples() {
        let contents = [
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_backpressure_guard","route_reason":"backpressure_attempts_exhausted"}}"#,
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"succeeded"}}"#,
        ];

        let signals = summarize_governor_history_signals(contents.iter().copied());
        assert_eq!(signals.final_status_failed_samples, vec![true, false]);
        assert_eq!(signals.backpressure_failure_samples, vec![true, false]);
        assert_eq!(
            signals
                .summary
                .failure_code_counts
                .get("safe_lane_plan_backpressure_guard")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn session_governor_engages_on_failed_final_status_threshold() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_session_governor_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_failed_final_status_threshold = 2;
        config
            .conversation
            .safe_lane_session_governor_backpressure_failure_threshold = 9;
        config
            .conversation
            .safe_lane_session_governor_force_no_replan = true;
        config
            .conversation
            .safe_lane_session_governor_force_node_max_attempts = 1;

        let mut summary = SafeLaneEventSummary::default();
        summary.final_status_counts.insert("failed".to_owned(), 2);

        let history = governor_history_with_summary(summary);
        let decision = decide_safe_lane_session_governor(&config, &history);
        assert!(decision.engaged);
        assert!(decision.failed_threshold_triggered);
        assert!(!decision.backpressure_threshold_triggered);
        assert!(decision.force_no_replan);
        assert_eq!(decision.forced_node_max_attempts, Some(1));
    }

    #[test]
    fn session_governor_engages_on_backpressure_threshold() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_session_governor_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_failed_final_status_threshold = 9;
        config
            .conversation
            .safe_lane_session_governor_backpressure_failure_threshold = 2;
        config
            .conversation
            .safe_lane_session_governor_force_node_max_attempts = 2;

        let mut summary = SafeLaneEventSummary::default();
        summary
            .failure_code_counts
            .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);
        summary.failure_code_counts.insert(
            "safe_lane_plan_verify_failed_backpressure_guard".to_owned(),
            1,
        );

        let history = governor_history_with_summary(summary);
        let decision = decide_safe_lane_session_governor(&config, &history);
        assert!(decision.engaged);
        assert!(!decision.failed_threshold_triggered);
        assert!(decision.backpressure_threshold_triggered);
        assert_eq!(decision.backpressure_failure_events, 2);
        assert_eq!(decision.forced_node_max_attempts, Some(2));
    }

    #[test]
    fn session_governor_stays_disabled_when_thresholds_not_reached() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_session_governor_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_failed_final_status_threshold = 3;
        config
            .conversation
            .safe_lane_session_governor_backpressure_failure_threshold = 2;

        let mut summary = SafeLaneEventSummary::default();
        summary.final_status_counts.insert("failed".to_owned(), 1);
        summary
            .failure_code_counts
            .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);

        let history = governor_history_with_summary(summary);
        let decision = decide_safe_lane_session_governor(&config, &history);
        assert!(!decision.engaged);
        assert!(!decision.force_no_replan);
        assert_eq!(decision.forced_node_max_attempts, None);
    }

    #[test]
    fn session_governor_engages_on_trend_threshold_when_counts_are_low() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_session_governor_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_failed_final_status_threshold = 9;
        config
            .conversation
            .safe_lane_session_governor_backpressure_failure_threshold = 9;
        config.conversation.safe_lane_session_governor_trend_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_trend_min_samples = 4;
        config
            .conversation
            .safe_lane_session_governor_trend_ewma_alpha = 0.5;
        config
            .conversation
            .safe_lane_session_governor_trend_failure_ewma_threshold = 0.60;
        config
            .conversation
            .safe_lane_session_governor_trend_backpressure_ewma_threshold = 0.70;

        let mut summary = SafeLaneEventSummary::default();
        summary.final_status_counts.insert("failed".to_owned(), 1);
        let history = SafeLaneGovernorHistorySignals {
            summary,
            final_status_failed_samples: vec![false, true, true, true],
            backpressure_failure_samples: vec![false, false, false, false],
        };

        let decision = decide_safe_lane_session_governor(&config, &history);
        assert!(decision.engaged);
        assert!(!decision.failed_threshold_triggered);
        assert!(!decision.backpressure_threshold_triggered);
        assert!(decision.trend_threshold_triggered);
        assert!(
            decision
                .trend_failure_ewma
                .map(|value| value > 0.60)
                .unwrap_or(false)
        );
    }

    #[test]
    fn session_governor_recovery_threshold_can_suppress_engagement() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_session_governor_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_failed_final_status_threshold = 1;
        config
            .conversation
            .safe_lane_session_governor_backpressure_failure_threshold = 9;
        config.conversation.safe_lane_session_governor_trend_enabled = true;
        config
            .conversation
            .safe_lane_session_governor_trend_min_samples = 4;
        config
            .conversation
            .safe_lane_session_governor_trend_ewma_alpha = 0.5;
        config
            .conversation
            .safe_lane_session_governor_trend_failure_ewma_threshold = 0.70;
        config
            .conversation
            .safe_lane_session_governor_recovery_success_streak = 3;
        config
            .conversation
            .safe_lane_session_governor_recovery_max_failure_ewma = 0.30;
        config
            .conversation
            .safe_lane_session_governor_recovery_max_backpressure_ewma = 0.10;

        let mut summary = SafeLaneEventSummary::default();
        summary.final_status_counts.insert("failed".to_owned(), 1);
        let history = SafeLaneGovernorHistorySignals {
            summary,
            final_status_failed_samples: vec![true, false, false, false, false],
            backpressure_failure_samples: vec![true, false, false, false, false],
        };

        let decision = decide_safe_lane_session_governor(&config, &history);
        assert!(decision.failed_threshold_triggered);
        assert!(!decision.trend_threshold_triggered);
        assert!(decision.recovery_threshold_triggered);
        assert_eq!(decision.recovery_success_streak, 4);
        assert!(!decision.engaged);
    }

    #[test]
    fn session_governor_route_override_marks_no_replan_terminal_reason() {
        let route = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "round_budget_exhausted",
        };
        let governor = SafeLaneSessionGovernorDecision {
            force_no_replan: true,
            ..SafeLaneSessionGovernorDecision::default()
        };
        let overridden = apply_safe_lane_session_governor_route_override(route, governor);
        assert_eq!(overridden.reason, "session_governor_no_replan");
    }

    #[test]
    fn terminal_verify_failure_uses_backpressure_error_code() {
        let failure = terminal_turn_failure_from_verify_failure(
            "retryable verify failure",
            true,
            "backpressure_attempts_exhausted",
        );
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_backpressure_guard"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    }

    #[test]
    fn terminal_verify_failure_uses_session_governor_error_code() {
        let failure = terminal_turn_failure_from_verify_failure(
            "retryable verify failure",
            true,
            "session_governor_no_replan",
        );
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_session_governor"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    }

    #[test]
    fn terminal_plan_failure_uses_session_governor_error_code() {
        let failure = PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 1,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        };
        let route = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: "session_governor_no_replan",
        };
        let result = terminal_turn_result_from_plan_failure_with_route(failure, route);
        let meta = result.failure().expect("failure metadata");
        assert_eq!(meta.code, "safe_lane_plan_session_governor_no_replan");
        assert_eq!(meta.kind, TurnFailureKind::NonRetryable);
    }
}
