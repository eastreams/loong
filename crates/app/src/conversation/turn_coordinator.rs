use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "memory-sqlite")]
use std::panic::AssertUnwindSafe;

use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use futures_util::FutureExt;
use loongclaw_contracts::{AuditEventKind, ExecutionPlane, PlaneTier};
use serde::Serialize;
use serde_json::{Value, json};
#[cfg(feature = "memory-sqlite")]
use tokio::runtime::Handle;
use tokio::sync::Mutex;
#[cfg(feature = "memory-sqlite")]
use tokio::time::{Duration, Instant, timeout};

#[path = "turn_coordinator/safe_lane_events.rs"]
mod safe_lane_events;
#[path = "turn_coordinator/safe_lane_execution.rs"]
mod safe_lane_execution;
#[path = "turn_coordinator/safe_lane_routing.rs"]
mod safe_lane_routing;
#[path = "turn_coordinator_support.rs"]
mod support;

use crate::CliResult;
#[cfg(test)]
use crate::KernelContext;
use crate::acp::{
    AcpConversationTurnEntryDecision, AcpConversationTurnExecutionOutcome,
    AcpConversationTurnOptions, AcpTurnEventSink, evaluate_acp_conversation_turn_entry_for_address,
    execute_acp_conversation_turn_for_address,
};
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::{
    DelegateChildExecutionPolicy, build_delegate_child_lifecycle_seed,
};
use crate::runtime_self_continuity;

use self::safe_lane_events::*;
use self::safe_lane_execution::*;
pub(crate) use self::safe_lane_routing::SafeLaneFailureRoute;
use self::safe_lane_routing::*;
use super::super::config::{LoongClawConfig, ToolConsentMode};
use super::ConversationSessionAddress;
use super::ProviderErrorMode;
use super::analytics::{
    SafeLaneEventSummary, TurnCheckpointProgressStatus as AnalyticsTurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, build_turn_checkpoint_repair_plan, summarize_safe_lane_history,
};
#[cfg(feature = "memory-sqlite")]
use super::announce::DelegateAnnounceSettings;
#[cfg(feature = "memory-sqlite")]
use super::approval_resolution::CoordinatorApprovalResolutionRuntime;
use super::context_engine::AssembledConversationContext;
#[cfg(feature = "memory-sqlite")]
use super::delegate_support::{
    enqueue_delegate_result_announce_with_memory_config,
    finalize_and_announce_delegate_child_terminal,
    finalize_async_delegate_spawn_failure_with_recovery, format_delegate_child_panic,
    next_delegate_child_depth_for_delegate, spawn_async_delegate_detached,
};
use super::ingress::ConversationIngressContext;
use super::lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
use super::persistence::{
    format_provider_error_reply, persist_acp_runtime_events, persist_conversation_event,
    persist_reply_turns_raw_with_mode, persist_reply_turns_with_mode, persist_tool_decision,
    persist_tool_outcome, provider_error_reply_body,
};
use super::plan_executor::{
    PlanExecutor, PlanNodeError, PlanNodeErrorKind, PlanNodeExecutor, PlanRunFailure,
    PlanRunReport, PlanRunStatus,
};
use super::plan_ir::{
    PLAN_GRAPH_VERSION, PlanBudget, PlanEdge, PlanGraph, PlanNode, PlanNodeKind, RiskTier,
};
use super::plan_verifier::{
    PlanVerificationContext, PlanVerificationFailureCode, PlanVerificationPolicy,
    PlanVerificationReport, verify_output,
};
use super::runtime::{
    AsyncDelegateSpawnRequest, BoxedDefaultConversationRuntime, ConversationRuntime,
    SessionContext, load_default_conversation_runtime,
};
use super::runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
use super::safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    classify_safe_lane_plan_failure,
};
#[cfg(feature = "memory-sqlite")]
use super::session_history::{
    AssistantHistoryLoadErrorCode, load_assistant_contents_from_session_window_detailed,
    load_latest_turn_checkpoint_entry, load_turn_checkpoint_history_snapshot,
};
#[cfg(feature = "memory-sqlite")]
use super::subagent::{
    ConstrainedSubagentExecution, ConstrainedSubagentMode, ConstrainedSubagentOwnerKind,
    ConstrainedSubagentTerminalReason,
};
use super::tool_discovery_state::{TOOL_DISCOVERY_REFRESHED_EVENT_NAME, ToolDiscoveryState};
use super::trust_projection::{
    emit_provider_failover_trust_event_if_needed, emit_runtime_binding_trust_event_if_needed,
};
use super::turn_budget::{
    EscalatingAttemptBudget, SafeLaneBackpressureBudget, SafeLaneContinuationBudgetDecision,
    SafeLaneFailureRouteReason, SafeLaneReplanBudget,
};
#[cfg(test)]
use super::turn_checkpoint::TurnCheckpointResultKind;
use super::turn_checkpoint::{
    ContextCompactionOutcome, TurnCheckpointDiagnostics, TurnCheckpointFailure,
    TurnCheckpointFailureStep, TurnCheckpointFinalizationProgress, TurnCheckpointIdentity,
    TurnCheckpointProgressStatus, TurnCheckpointRecoveryAssessment,
    TurnCheckpointRepairResumeInput, TurnCheckpointRequest, TurnCheckpointSnapshot,
    TurnCheckpointStage, TurnCheckpointTailRepairOutcome, TurnCheckpointTailRepairReason,
    TurnCheckpointTailRepairRuntimeProbe, TurnCheckpointTailRepairSource,
    TurnCheckpointTailRepairStatus, TurnCheckpointTailRuntimeEligibility,
    TurnFinalizationCheckpoint, TurnLaneExecutionSnapshot, TurnPreparationSnapshot,
    TurnReplyCheckpoint, checkpoint_context_fingerprint_sha256, persist_turn_checkpoint_event,
    persist_turn_checkpoint_event_value, restore_analytics_turn_checkpoint_progress_status,
    turn_checkpoint_result_kind,
};
use super::turn_engine::{
    AppToolDispatcher, DefaultAppToolDispatcher, ProviderTurn, ToolBatchExecutionIntentStatus,
    ToolBatchExecutionTrace, ToolExecutionPreflight, ToolIntent, TurnEngine, TurnFailure,
    TurnFailureKind, TurnResult, TurnValidation, effective_result_tool_name,
};
use super::turn_observer::{
    ConversationTurnObserverHandle, ConversationTurnPhase, ConversationTurnPhaseEvent,
    ConversationTurnToolEvent, build_observer_streaming_token_callback,
};
#[cfg(feature = "memory-sqlite")]
use super::turn_shared::{ApprovalPromptActionId, parse_approval_prompt_action_input};
use super::turn_shared::{
    ProviderTurnRequestAction, ReplyPersistenceMode, ToolDrivenFollowupPayload,
    ToolDrivenReplyBaseDecision, ToolDrivenReplyPhase,
    build_tool_driven_followup_tail_with_request_summary, build_tool_loop_guard_tail,
    decide_provider_turn_request_action, effective_followup_tool_name,
    format_approval_required_reply, next_conversation_turn_id, reduce_followup_payload_for_model,
    request_completion_with_raw_fallback, summarize_provider_lane_tool_request,
    summarize_single_tool_followup_request, tool_driven_followup_payload,
    tool_loop_circuit_breaker_reply, tool_result_contains_truncation_signal,
    user_requested_raw_tool_output,
};
#[cfg(test)]
use super::turn_shared::{ReplyResolutionMode, ToolDrivenFollowupKind};
#[cfg(feature = "memory-sqlite")]
use crate::conversation::workspace_isolation::{
    DelegateWorkspaceCleanupResult, cleanup_delegate_workspace_root,
    cleanup_prepared_delegate_workspace_root, prepare_delegate_workspace_root,
};
#[cfg(all(feature = "memory-sqlite", test))]
use crate::session::recovery::RECOVERY_EVENT_KIND;
#[cfg(all(test, feature = "memory-sqlite"))]
use crate::session::repository::FinalizeSessionTerminalRequest;
#[cfg(all(test, feature = "memory-sqlite"))]
use crate::session::repository::TransitionApprovalRequestIfCurrentRequest;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestStatus, NewSessionEvent, NewSessionRecord, SessionKind,
    SessionRepository, SessionState,
};
use support::{
    ProviderTurnPreparation, ProviderTurnReplyTailPhase, ProviderTurnSessionState,
    emit_async_delegate_child_queued_event, emit_discovery_first_event, emit_prompt_frame_event,
    estimate_tokens_for_messages, inject_delegate_workspace_metadata,
    split_delegate_workspace_cleanup, summarize_discovery_first_followup_turn,
};

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn emit_async_delegate_child_terminal_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    phase: &'static str,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    duration_ms: u64,
    turn_count: Option<usize>,
    error: Option<&str>,
    final_output: Option<&str>,
    workspace_root: Option<&std::path::Path>,
    workspace_retained: Option<bool>,
    binding: ConversationRuntimeBinding<'_>,
) {
    support::emit_async_delegate_child_terminal_event(
        runtime,
        parent_session_id,
        child_session_id,
        child_label,
        profile,
        phase,
        isolation,
        duration_ms,
        turn_count,
        error,
        final_output,
        workspace_root,
        workspace_retained,
        binding,
    )
    .await;
}

#[derive(Default)]
pub struct ConversationTurnCoordinator;

const PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING: &str =
    "production conversation runtime requires kernel-bound execution";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionReport {
    pub status: AnalyticsTurnCheckpointProgressStatus,
    pub estimated_tokens_before: Option<usize>,
    pub estimated_tokens_after: Option<usize>,
}

impl ContextCompactionReport {
    pub fn status_label(&self) -> &'static str {
        match self.status {
            AnalyticsTurnCheckpointProgressStatus::Pending => "pending",
            AnalyticsTurnCheckpointProgressStatus::Skipped => "skipped",
            AnalyticsTurnCheckpointProgressStatus::Completed => "completed",
            AnalyticsTurnCheckpointProgressStatus::Failed => "failed",
            AnalyticsTurnCheckpointProgressStatus::FailedOpen => "failed_open",
        }
    }

    pub fn was_applied(&self) -> bool {
        matches!(
            self.status,
            AnalyticsTurnCheckpointProgressStatus::Completed
        )
    }

    pub fn was_skipped(&self) -> bool {
        matches!(self.status, AnalyticsTurnCheckpointProgressStatus::Skipped)
    }

    pub fn was_failed_open(&self) -> bool {
        matches!(
            self.status,
            AnalyticsTurnCheckpointProgressStatus::FailedOpen
        )
    }
}

fn require_production_kernel_binding<'a>(
    binding: ConversationRuntimeBinding<'a>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ConversationRuntimeBinding<'a>> {
    if binding.is_kernel_bound() {
        return Ok(binding);
    }

    let failed_event = ConversationTurnPhaseEvent::failed();
    observe_turn_phase(observer, failed_event);

    let error = PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING.to_owned();
    Err(error)
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SafeLaneGovernorHistoryLoadStatus {
    #[default]
    Disabled,
    Loaded,
    Unavailable,
}

impl SafeLaneGovernorHistoryLoadStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Loaded => "loaded",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct SafeLaneSessionGovernorDecision {
    engaged: bool,
    history_window_turns: usize,
    history_load_status: SafeLaneGovernorHistoryLoadStatus,
    history_load_error: Option<AssistantHistoryLoadErrorCode>,
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
    fn as_json(&self) -> Value {
        json!({
            "engaged": self.engaged,
            "history_window_turns": self.history_window_turns,
            "history_load_status": self.history_load_status.as_str(),
            "history_load_error": self.history_load_error.map(|error| error.as_str()),
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
    history_load_status: SafeLaneGovernorHistoryLoadStatus,
    history_load_error: Option<AssistantHistoryLoadErrorCode>,
    summary: SafeLaneEventSummary,
    final_status_failed_samples: Vec<bool>,
    backpressure_failure_samples: Vec<bool>,
}

#[derive(Debug, Clone)]
struct SafeLanePlanLoopState {
    governor: SafeLaneSessionGovernorDecision,
    replan_budget: SafeLaneReplanBudget,
    tool_node_attempt_budget: EscalatingAttemptBudget,
    plan_start_tool_index: usize,
    seed_tool_outputs: Vec<String>,
    metrics: SafeLaneExecutionMetrics,
    adaptive_verify_policy: SafeLaneAdaptiveVerifyPolicyState,
}

impl SafeLanePlanLoopState {
    fn new(config: &LoongClawConfig, governor: SafeLaneSessionGovernorDecision) -> Self {
        let force_no_replan = governor.force_no_replan;
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

        Self {
            governor,
            replan_budget: SafeLaneReplanBudget::new(if force_no_replan {
                0
            } else {
                config.conversation.safe_lane_replan_max_rounds
            }),
            tool_node_attempt_budget: EscalatingAttemptBudget::new(
                tool_node_max_attempts,
                max_node_attempts,
            ),
            plan_start_tool_index: 0,
            seed_tool_outputs: Vec::new(),
            metrics: SafeLaneExecutionMetrics::default(),
            adaptive_verify_policy: SafeLaneAdaptiveVerifyPolicyState::default(),
        }
    }

    fn refresh_verify_policy(&mut self, config: &LoongClawConfig) -> Option<usize> {
        let next_min_anchor_matches =
            compute_safe_lane_verify_min_anchor_matches(config, self.metrics.verify_failures);
        if next_min_anchor_matches == self.adaptive_verify_policy.min_anchor_matches {
            return None;
        }
        self.adaptive_verify_policy.min_anchor_matches = next_min_anchor_matches;
        (next_min_anchor_matches > 0).then_some(next_min_anchor_matches)
    }

    fn note_round_started(&mut self) {
        self.metrics.rounds_started = self.metrics.rounds_started.saturating_add(1);
    }

    fn record_round_execution(&mut self, report: &PlanRunReport, stats: SafeLaneToolOutputStats) {
        self.metrics.total_attempts_used = self
            .metrics
            .total_attempts_used
            .saturating_add(report.attempts_used as u64);
        self.metrics.record_tool_output_stats(stats);
    }

    fn note_round_succeeded(&mut self) {
        self.metrics.rounds_succeeded = self.metrics.rounds_succeeded.saturating_add(1);
    }

    fn note_round_failed(&mut self) {
        self.metrics.rounds_failed = self.metrics.rounds_failed.saturating_add(1);
    }

    fn note_verify_failure(&mut self) {
        self.metrics.verify_failures = self.metrics.verify_failures.saturating_add(1);
    }

    fn note_replan(
        &mut self,
        next_plan_start_tool_index: usize,
        next_seed_tool_outputs: Vec<String>,
    ) {
        self.plan_start_tool_index = next_plan_start_tool_index;
        self.seed_tool_outputs = next_seed_tool_outputs;
        self.metrics.replans_triggered = self.metrics.replans_triggered.saturating_add(1);
    }

    fn advance_round(&mut self) {
        self.replan_budget = self.replan_budget.after_replan();
        self.tool_node_attempt_budget = self.tool_node_attempt_budget.after_retry();
    }

    fn round(&self) -> u8 {
        self.replan_budget.current_round()
    }

    fn max_rounds(&self) -> u8 {
        self.replan_budget.max_replans()
    }

    fn tool_node_max_attempts(&self) -> u8 {
        self.tool_node_attempt_budget.current_limit()
    }

    fn max_node_attempts(&self) -> u8 {
        self.tool_node_attempt_budget.max_limit()
    }
}

#[derive(Debug, Clone)]
struct SafeLaneRoundExecution {
    report: PlanRunReport,
    tool_outputs: Vec<String>,
    tool_output_stats: SafeLaneToolOutputStats,
}

#[derive(Debug, Clone)]
struct ProviderTurnLanePlan {
    decision: LaneDecision,
    max_tool_steps: usize,
}

impl ProviderTurnLanePlan {
    fn from_user_input(config: &LoongClawConfig, user_input: &str) -> Self {
        let decision = if config.conversation.hybrid_lane_enabled {
            lane_policy_from_config(config).decide(user_input)
        } else {
            disabled_lane_decision(user_input)
        };
        let max_tool_steps = match decision.lane {
            ExecutionLane::Fast => config.conversation.fast_lane_max_tool_steps(),
            ExecutionLane::Safe => config.conversation.safe_lane_max_tool_steps(),
        };

        Self {
            decision,
            max_tool_steps,
        }
    }

    fn should_use_safe_lane_plan_path(
        &self,
        config: &LoongClawConfig,
        turn: &ProviderTurn,
    ) -> bool {
        config.conversation.safe_lane_plan_execution_enabled
            && matches!(self.decision.lane, ExecutionLane::Safe)
            && !turn.tool_intents.is_empty()
    }
}

#[derive(Debug, Clone)]
struct ProviderTurnLaneExecution {
    lane: ExecutionLane,
    assistant_preface: String,
    provider_usage: Option<Value>,
    had_tool_intents: bool,
    tool_request_summary: Option<String>,
    discovery_search_turn: bool,
    search_tool_intents: usize,
    supports_provider_turn_followup: bool,
    raw_tool_output_requested: bool,
    turn_result: TurnResult,
    safe_lane_terminal_route: Option<SafeLaneFailureRoute>,
    tool_events: Vec<ConversationTurnToolEvent>,
}

impl ProviderTurnLaneExecution {
    fn checkpoint(&self) -> TurnLaneExecutionSnapshot {
        TurnLaneExecutionSnapshot {
            lane: self.lane,
            had_tool_intents: self.had_tool_intents,
            tool_request_summary: self.tool_request_summary.clone(),
            raw_tool_output_requested: self.raw_tool_output_requested,
            result_kind: turn_checkpoint_result_kind(&self.turn_result),
            safe_lane_terminal_route: self.safe_lane_terminal_route,
        }
    }

    fn reply_phase(&self) -> ToolDrivenReplyPhase {
        ToolDrivenReplyPhase::new(
            self.assistant_preface.as_str(),
            self.had_tool_intents,
            self.raw_tool_output_requested,
            &self.turn_result,
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct ProviderTurnLoopPolicy {
    max_total_tool_calls: usize,
    max_consecutive_same_tool: usize,
}

impl ProviderTurnLoopPolicy {
    fn from_config(config: &LoongClawConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_total_tool_calls: turn_loop.max_total_tool_calls.max(1),
            max_consecutive_same_tool: turn_loop.max_consecutive_same_tool.max(1),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ProviderTurnLoopState {
    total_tool_calls: usize,
    consecutive_same_tool: usize,
    last_tool_name: Option<String>,
    warned_same_tool_key: Option<String>,
}

#[derive(Debug, Clone)]
enum ProviderTurnLoopVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

impl ProviderTurnLoopState {
    fn circuit_breaker_reply(
        &self,
        policy: &ProviderTurnLoopPolicy,
        next_tool_calls: usize,
    ) -> Option<String> {
        let prospective_total = self.total_tool_calls.saturating_add(next_tool_calls);
        tool_loop_circuit_breaker_reply(prospective_total, policy.max_total_tool_calls)
    }

    fn observe_turn(
        &mut self,
        policy: &ProviderTurnLoopPolicy,
        turn: &ProviderTurn,
    ) -> Option<ProviderTurnLoopVerdict> {
        let tool_intent_count = turn.tool_intents.len();
        self.total_tool_calls = self.total_tool_calls.saturating_add(tool_intent_count);
        if tool_intent_count == 0 {
            self.warned_same_tool_key = None;
            return None;
        }

        let tool_name_signature = provider_turn_tool_name_signature(&turn.tool_intents);
        if self.last_tool_name.as_deref() == Some(tool_name_signature.as_str()) {
            self.consecutive_same_tool += 1;
        } else {
            self.last_tool_name = Some(tool_name_signature.clone());
            self.consecutive_same_tool = 1;
            self.warned_same_tool_key = None;
        }

        if self.consecutive_same_tool < policy.max_consecutive_same_tool {
            self.warned_same_tool_key = None;
            return Some(ProviderTurnLoopVerdict::Continue);
        }

        let reason_key = format!("consecutive_same_tool:{tool_name_signature}");
        let reason = format!(
            "consecutive_same_tool: {tool_name_signature} called {} times in a row (limit={})",
            self.consecutive_same_tool, policy.max_consecutive_same_tool
        );

        if self.warned_same_tool_key.as_deref() == Some(reason_key.as_str()) {
            Some(ProviderTurnLoopVerdict::HardStop { reason })
        } else {
            self.warned_same_tool_key = Some(reason_key);
            Some(ProviderTurnLoopVerdict::InjectWarning { reason })
        }
    }
}

fn provider_turn_tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

#[derive(Debug, Clone)]
struct ProviderTurnContinuePhase {
    request: TurnCheckpointRequest,
    lane_execution: ProviderTurnLaneExecution,
    reply_phase: ToolDrivenReplyPhase,
    loop_verdict: Option<ProviderTurnLoopVerdict>,
    followup_config: LoongClawConfig,
    ingress: Option<ConversationIngressContext>,
}

impl ProviderTurnContinuePhase {
    fn new(
        tool_intents: usize,
        lane_execution: ProviderTurnLaneExecution,
        loop_verdict: Option<ProviderTurnLoopVerdict>,
        followup_config: LoongClawConfig,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let reply_phase = lane_execution.reply_phase();
        Self {
            request: TurnCheckpointRequest::Continue { tool_intents },
            lane_execution,
            reply_phase,
            loop_verdict,
            followup_config,
            ingress: ingress.cloned(),
        }
    }

    fn checkpoint(
        &self,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        reply: &str,
    ) -> TurnCheckpointSnapshot {
        build_resolved_provider_checkpoint(
            preparation,
            user_input,
            Some(reply),
            self.request.clone(),
            Some(self.lane_execution.checkpoint()),
            Some(TurnReplyCheckpoint::from_phase(&self.reply_phase)),
            TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
        )
    }

    fn tool_intent_count(&self) -> usize {
        match self.request {
            TurnCheckpointRequest::Continue { tool_intents } => tool_intents,
            TurnCheckpointRequest::FinalizeInlineProviderError
            | TurnCheckpointRequest::ReturnError => 0,
        }
    }

    fn loop_warning_reason(&self) -> Option<&str> {
        match self.loop_verdict.as_ref() {
            Some(ProviderTurnLoopVerdict::InjectWarning { reason }) => Some(reason.as_str()),
            _ => None,
        }
    }

    fn hard_stop_reason(&self) -> Option<&str> {
        match self.loop_verdict.as_ref() {
            Some(ProviderTurnLoopVerdict::HardStop { reason }) => Some(reason.as_str()),
            _ => None,
        }
    }

    async fn resolve<R: ConversationRuntime + ?Sized>(
        &self,
        runtime: &R,
        session_id: &str,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        turn_loop_policy: &ProviderTurnLoopPolicy,
        turn_loop_state: &mut ProviderTurnLoopState,
        remaining_provider_rounds: usize,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> ResolvedProviderTurn {
        resolve_provider_turn_reply(
            runtime,
            &self.followup_config,
            session_id,
            preparation,
            self,
            user_input,
            turn_loop_policy,
            turn_loop_state,
            remaining_provider_rounds,
            binding,
            self.ingress.as_ref(),
            observer,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedProviderTurn {
    PersistReply(ResolvedProviderReply),
    ReturnError(ResolvedProviderError),
}

impl ResolvedProviderTurn {
    fn persist_reply(
        reply: String,
        usage: Option<Value>,
        checkpoint: TurnCheckpointSnapshot,
    ) -> Self {
        Self::PersistReply(ResolvedProviderReply {
            reply,
            usage,
            checkpoint,
        })
    }

    fn return_error(error: String, checkpoint: TurnCheckpointSnapshot) -> Self {
        Self::ReturnError(ResolvedProviderError { error, checkpoint })
    }

    #[cfg(test)]
    fn checkpoint(&self) -> &TurnCheckpointSnapshot {
        match self {
            Self::PersistReply(reply) => &reply.checkpoint,
            Self::ReturnError(error) => &error.checkpoint,
        }
    }

    fn terminal_phase<'a>(
        &'a self,
        session: &ProviderTurnSessionState,
    ) -> ProviderTurnTerminalPhase<'a> {
        match self {
            Self::PersistReply(reply) => {
                ProviderTurnTerminalPhase::PersistReply(ProviderTurnPersistReplyPhase {
                    checkpoint: &reply.checkpoint,
                    tail_phase: ProviderTurnReplyTailPhase::from_session(
                        session,
                        reply.reply.as_str(),
                    ),
                    usage: reply.usage.clone(),
                })
            }
            Self::ReturnError(error) => {
                ProviderTurnTerminalPhase::ReturnError(ProviderTurnReturnErrorPhase {
                    checkpoint: &error.checkpoint,
                    error: error.error.as_str(),
                })
            }
        }
    }

    #[cfg(test)]
    fn reply_text(&self) -> Option<&str> {
        match self {
            Self::PersistReply(reply) => Some(reply.reply.as_str()),
            Self::ReturnError(_) => None,
        }
    }

    fn provider_error_text(&self) -> Option<&str> {
        match self {
            Self::PersistReply(reply) => provider_error_reply_body(reply.reply.as_str()),
            Self::ReturnError(error) => Some(error.error.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProviderReply {
    reply: String,
    usage: Option<Value>,
    checkpoint: TurnCheckpointSnapshot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationTurnOutcome {
    pub reply: String,
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProviderError {
    error: String,
    checkpoint: TurnCheckpointSnapshot,
}

#[derive(Debug)]
enum ProviderTurnTerminalPhase<'a> {
    PersistReply(ProviderTurnPersistReplyPhase<'a>),
    ReturnError(ProviderTurnReturnErrorPhase<'a>),
}

impl<'a> ProviderTurnTerminalPhase<'a> {
    async fn apply<R: ConversationRuntime + ?Sized>(
        self,
        config: &LoongClawConfig,
        runtime: &R,
        session_id: &str,
        user_input: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ConversationTurnOutcome> {
        match self {
            Self::PersistReply(phase) => {
                finalize_provider_turn_reply(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    &phase.tail_phase,
                    phase.usage,
                    phase.checkpoint,
                    binding,
                )
                .await
            }
            Self::ReturnError(phase) => {
                persist_resolved_provider_error_checkpoint(
                    runtime,
                    session_id,
                    phase.checkpoint,
                    binding,
                )
                .await?;
                Err(phase.error.to_owned())
            }
        }
    }
}

#[derive(Debug)]
struct ProviderTurnPersistReplyPhase<'a> {
    checkpoint: &'a TurnCheckpointSnapshot,
    tail_phase: ProviderTurnReplyTailPhase,
    usage: Option<Value>,
}

#[derive(Debug)]
struct ProviderTurnReturnErrorPhase<'a> {
    checkpoint: &'a TurnCheckpointSnapshot,
    error: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderTurnRequestTerminalPhase {
    PersistInlineProviderError { reply: String },
    ReturnError { error: String },
}

impl ProviderTurnRequestTerminalPhase {
    fn persist_inline_provider_error(reply: String) -> Self {
        Self::PersistInlineProviderError { reply }
    }

    fn return_error(error: String) -> Self {
        Self::ReturnError { error }
    }

    fn resolve(
        self,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
    ) -> ResolvedProviderTurn {
        match self {
            Self::PersistInlineProviderError { reply } => {
                let checkpoint = build_resolved_provider_checkpoint(
                    preparation,
                    user_input,
                    Some(reply.as_str()),
                    TurnCheckpointRequest::FinalizeInlineProviderError,
                    None,
                    None,
                    TurnFinalizationCheckpoint::persist_reply(
                        ReplyPersistenceMode::InlineProviderError,
                    ),
                );
                ResolvedProviderTurn::persist_reply(reply, None, checkpoint)
            }
            Self::ReturnError { error } => {
                let checkpoint = build_resolved_provider_checkpoint(
                    preparation,
                    user_input,
                    None,
                    TurnCheckpointRequest::ReturnError,
                    None,
                    None,
                    TurnFinalizationCheckpoint::ReturnError,
                );
                ResolvedProviderTurn::return_error(error, checkpoint)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct SafeLaneTurnOutcome {
    result: TurnResult,
    terminal_route: Option<SafeLaneFailureRoute>,
}

impl SafeLaneTurnOutcome {
    fn without_terminal_route(result: TurnResult) -> Self {
        Self {
            result,
            terminal_route: None,
        }
    }

    fn with_terminal_route(result: TurnResult, terminal_route: SafeLaneFailureRoute) -> Self {
        Self {
            result,
            terminal_route: Some(terminal_route),
        }
    }
}

fn build_resolved_provider_checkpoint(
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    reply_text: Option<&str>,
    request: TurnCheckpointRequest,
    lane: Option<TurnLaneExecutionSnapshot>,
    reply: Option<TurnReplyCheckpoint>,
    finalization: TurnFinalizationCheckpoint,
) -> TurnCheckpointSnapshot {
    TurnCheckpointSnapshot {
        identity: reply_text
            .map(|assistant_reply| TurnCheckpointIdentity::from_turn(user_input, assistant_reply)),
        preparation: preparation.checkpoint(),
        request,
        lane,
        reply,
        finalization,
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingApprovalInputDecision {
    RunOnce,
    SessionAuto,
    SessionFull,
    Cancel,
}

#[cfg(feature = "memory-sqlite")]
impl PendingApprovalInputDecision {
    fn approval_decision(self) -> ApprovalDecision {
        match self {
            Self::RunOnce | Self::SessionAuto | Self::SessionFull => ApprovalDecision::ApproveOnce,
            Self::Cancel => ApprovalDecision::Deny,
        }
    }

    fn session_mode(self) -> Option<ToolConsentMode> {
        match self {
            Self::RunOnce | Self::Cancel => None,
            Self::SessionAuto => Some(ToolConsentMode::Auto),
            Self::SessionFull => Some(ToolConsentMode::Full),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn normalize_pending_approval_control_input(input: &str) -> String {
    super::turn_shared::normalize_approval_prompt_control_input(input)
}

#[cfg(feature = "memory-sqlite")]
fn parse_pending_approval_input_decision(input: &str) -> Option<PendingApprovalInputDecision> {
    match parse_approval_prompt_action_input(input)? {
        ApprovalPromptActionId::Yes => Some(PendingApprovalInputDecision::RunOnce),
        ApprovalPromptActionId::Auto => Some(PendingApprovalInputDecision::SessionAuto),
        ApprovalPromptActionId::Full => Some(PendingApprovalInputDecision::SessionFull),
        ApprovalPromptActionId::Esc => Some(PendingApprovalInputDecision::Cancel),
    }
}

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub fn new() -> Self {
        Self
    }

    pub(crate) async fn compact_session(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ContextCompactionReport> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.compact_session_with_runtime(config, session_id, &runtime, binding)
            .await
    }

    pub async fn compact_production_session(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ContextCompactionReport> {
        let production_binding = require_production_kernel_binding(binding, None)?;
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.compact_session_with_runtime(config, session_id, &runtime, production_binding)
            .await
    }

    pub(crate) async fn compact_session_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ContextCompactionReport> {
        if let Some(kernel_ctx) = binding.kernel_context() {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }

        let session_context = runtime.session_context(config, session_id, binding)?;
        let tool_view = session_context.tool_view.clone();
        let before_messages = runtime
            .build_messages(config, session_id, true, &tool_view, binding)
            .await?;
        let estimated_tokens_before = estimate_tokens(&before_messages);
        let compaction_outcome = maybe_compact_context(
            config,
            runtime,
            session_id,
            &before_messages,
            estimated_tokens_before,
            binding,
            true,
        )
        .await?;

        let mut status = compaction_outcome.checkpoint_status();
        let mut estimated_tokens_after = estimated_tokens_before;

        if compaction_outcome == ContextCompactionOutcome::Completed {
            match runtime
                .build_messages(config, session_id, true, &tool_view, binding)
                .await
            {
                Ok(after_messages) => {
                    let did_change = before_messages != after_messages;
                    let next_estimated_tokens = estimate_tokens(&after_messages);

                    estimated_tokens_after = next_estimated_tokens;

                    if !did_change {
                        status = TurnCheckpointProgressStatus::Skipped;
                    }
                }
                Err(_error) => {
                    status = TurnCheckpointProgressStatus::Skipped;
                    estimated_tokens_after = estimated_tokens_before;
                }
            }
        }

        let report = ContextCompactionReport {
            status: analytics_turn_checkpoint_progress_status(status),
            estimated_tokens_before,
            estimated_tokens_after,
        };

        Ok(report)
    }

    pub(crate) async fn handle_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_ingress(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id(session_id);
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            &address,
            user_input,
            error_mode,
            &runtime,
            &acp_options,
            binding,
            ingress,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_acp_options(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_address_and_acp_options(
            config,
            &address,
            user_input,
            error_mode,
            acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn repair_turn_checkpoint_tail(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.repair_turn_checkpoint_tail_with_runtime(config, session_id, &runtime, binding)
            .await
    }

    pub async fn repair_production_turn_checkpoint_tail(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        let production_binding = require_production_kernel_binding(binding, None)?;
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.repair_turn_checkpoint_tail_with_runtime(
            config,
            session_id,
            &runtime,
            production_binding,
        )
        .await
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            &runtime,
            binding,
        )
        .await
    }

    pub(crate) async fn load_production_turn_checkpoint_diagnostics(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let production_binding = require_production_kernel_binding(binding, None)?;

        self.load_turn_checkpoint_diagnostics(config, session_id, production_binding)
            .await
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config, session_id, limit, &runtime, binding,
        )
        .await
    }

    pub(crate) async fn load_production_turn_checkpoint_diagnostics_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let production_binding = require_production_kernel_binding(binding, None)?;

        self.load_turn_checkpoint_diagnostics_with_limit(
            config,
            session_id,
            limit,
            production_binding,
        )
        .await
    }

    pub(crate) async fn probe_turn_checkpoint_tail_runtime_gate(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            &runtime,
            binding,
        )
        .await
    }

    pub async fn probe_production_turn_checkpoint_tail_runtime_gate(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let production_binding = require_production_kernel_binding(binding, None)?;
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            &runtime,
            production_binding,
        )
        .await
    }

    pub(crate) async fn probe_turn_checkpoint_tail_runtime_gate_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config, session_id, limit, &runtime, binding,
        )
        .await
    }

    pub async fn probe_production_turn_checkpoint_tail_runtime_gate_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let production_binding = require_production_kernel_binding(binding, None)?;
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            limit,
            &runtime,
            production_binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_acp_event_sink(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_event_sink(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_address_and_acp_options(
            config,
            address,
            user_input,
            error_mode,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_ingress(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            binding,
            ingress,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, None)?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            binding,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_ingress_and_observer(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        let runtime = Self::build_default_runtime_or_observe_failure(config, observer.as_ref())?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            binding,
            ingress,
            observer,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_address_and_acp_options_and_observer(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_address_and_acp_options_and_ingress_and_observer(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            binding,
            None,
            observer,
        )
        .await
    }

    pub async fn handle_production_turn_with_address_and_acp_options_and_observer(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        let observer_ref = observer.as_ref();
        let production_binding = require_production_kernel_binding(binding, observer_ref)?;

        self.handle_turn_with_address_and_acp_options_and_observer(
            config,
            address,
            user_input,
            error_mode,
            acp_options,
            production_binding,
            observer,
        )
        .await
    }

    fn build_default_runtime_or_observe_failure(
        config: &LoongClawConfig,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<BoxedDefaultConversationRuntime> {
        // Keep runtime-construction failures visible to the turn observer so
        // operator surfaces receive the same failed phase signal as execution
        // errors later in the turn pipeline.
        let runtime_result = load_default_conversation_runtime(config);
        let runtime = match runtime_result {
            Ok(runtime) => runtime,
            Err(error) => {
                let failed_event = ConversationTurnPhaseEvent::failed();
                observe_turn_phase(observer, failed_event);
                return Err(error);
            }
        };
        Ok(runtime)
    }

    pub(crate) async fn handle_turn_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_runtime_and_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_ingress<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            &address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
            ingress,
        )
        .await
    }

    pub(crate) async fn repair_turn_checkpoint_tail_with_runtime<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            let Some(entry) = load_latest_turn_checkpoint_entry(
                session_id,
                config.memory.sliding_window,
                binding,
                &memory_config,
            )
            .await?
            else {
                return Ok(TurnCheckpointTailRepairOutcome::no_checkpoint());
            };

            repair_turn_checkpoint_tail_entry(config, runtime, session_id, &entry, binding).await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, binding);
            Err("turn checkpoint repair unavailable: memory-sqlite feature disabled".to_owned())
        }
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            let (summary, latest_entry) =
                load_turn_checkpoint_history_snapshot(session_id, limit, binding, &memory_config)
                    .await?
                    .into_summary_and_latest_entry();
            let recovery = TurnCheckpointRecoveryAssessment::from_summary(&summary);
            let runtime_probe = match recovery.action() {
                TurnCheckpointRecoveryAction::None
                | TurnCheckpointRecoveryAction::InspectManually => None,
                TurnCheckpointRecoveryAction::RunAfterTurn
                | TurnCheckpointRecoveryAction::RunCompaction
                | TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction => {
                    match latest_entry.as_ref() {
                        Some(entry) => {
                            probe_turn_checkpoint_tail_runtime_gate_entry(
                                config, runtime, session_id, entry, binding,
                            )
                            .await?
                        }
                        None => None,
                    }
                }
            };
            Ok(TurnCheckpointDiagnostics::new(
                summary,
                recovery,
                runtime_probe,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, limit, runtime, binding);
            Err(
                "turn checkpoint diagnostics unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }

    pub(crate) async fn probe_turn_checkpoint_tail_runtime_gate_with_runtime<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            runtime,
            binding,
        )
        .await
    }

    pub(crate) async fn probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        #[cfg(feature = "memory-sqlite")]
        {
            probe_turn_checkpoint_tail_runtime_gate_entry_with_limit(
                config, runtime, session_id, limit, binding,
            )
            .await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, binding);
            Err(
                "turn checkpoint runtime probe unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }

    pub(crate) async fn handle_turn_with_runtime_and_acp_options<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let address = ConversationSessionAddress::from_session_id(session_id);
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            &address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_acp_event_sink<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_runtime_and_acp_options(
            config,
            session_id,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            None,
        )
        .await
    }

    async fn handle_turn_with_runtime_and_address_and_acp_options_and_ingress<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            ingress,
            None,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_outcome(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            binding,
            ingress,
            observer,
        )
        .await
        .map(|outcome| outcome.reply)
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_outcome<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<ConversationTurnOutcome> {
        let turn_result: CliResult<(ConversationTurnOutcome, bool)> = async {
            let session_id = address.session_id.as_str();
            #[cfg(feature = "memory-sqlite")]
            if let Some(reply) = self
                .maybe_handle_pending_approval_control_turn(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    error_mode,
                    binding,
                    observer.as_ref(),
                )
                .await?
            {
                return Ok((ConversationTurnOutcome { reply, usage: None }, false));
            }
            let preparing_event = ConversationTurnPhaseEvent::preparing();
            observe_turn_phase(observer.as_ref(), preparing_event);

            let acp_entry_decision =
                evaluate_acp_conversation_turn_entry_for_address(config, address, acp_options)?;
            match acp_entry_decision {
                AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled => {
                    let error = "ACP is disabled by policy (`acp.enabled=false`)".to_owned();
                    let turn_result = match error_mode {
                        ProviderErrorMode::Propagate => Err(error),
                        ProviderErrorMode::InlineMessage => {
                            let synthetic = format_provider_error_reply(&error);
                            persist_reply_turns_raw_with_mode(
                                runtime,
                                session_id,
                                user_input,
                                &synthetic,
                                ReplyPersistenceMode::InlineProviderError,
                                binding,
                            )
                            .await?;
                            Ok(synthetic)
                        }
                    };
                    let reply = turn_result?;
                    return Ok((ConversationTurnOutcome { reply, usage: None }, true));
                }
                AcpConversationTurnEntryDecision::RouteViaAcp => {
                    let reply = self
                        .handle_turn_via_acp(
                            config,
                            address,
                            user_input,
                            error_mode,
                            runtime,
                            acp_options,
                            binding,
                        )
                        .await?;
                    return Ok((ConversationTurnOutcome { reply, usage: None }, true));
                }
                AcpConversationTurnEntryDecision::StayOnProvider => {}
            }

            if let Some(kernel_ctx) = binding.kernel_context() {
                runtime.bootstrap(config, session_id, kernel_ctx).await?;
            }

            let session_context = runtime.session_context(config, session_id, binding)?;
            let tool_view = session_context.tool_view.clone();
            let visible_ingress = ingress.filter(|value| value.has_contextual_hints());
            emit_turn_ingress_event(runtime, session_id, visible_ingress, binding).await;

            let turn_id = next_conversation_turn_id();
            let assembled_context = runtime
                .build_context(config, session_id, true, binding)
                .await?;
            let preparation = ProviderTurnPreparation::from_assembled_context_with_turn_id(
                config,
                assembled_context,
                user_input,
                turn_id.as_str(),
                visible_ingress,
            );
            let context_message_count = preparation.session.messages.len();
            let context_estimated_tokens = preparation.session.estimated_tokens;
            let initial_request_event = ConversationTurnPhaseEvent::requesting_provider(
                1,
                context_message_count,
                context_estimated_tokens,
            );
            observe_turn_phase(
                observer.as_ref(),
                ConversationTurnPhaseEvent::context_ready(
                    context_message_count,
                    context_estimated_tokens,
                ),
            );
            observe_turn_phase(observer.as_ref(), initial_request_event);
            emit_prompt_frame_event(
                runtime,
                session_id,
                1,
                "initial",
                preparation.session.prompt_frame_summary(),
                binding,
            )
            .await;

            let provider_turn_result = request_provider_turn_with_observer(
                config,
                runtime,
                session_id,
                preparation.turn_id.as_str(),
                &preparation.session.messages,
                &tool_view,
                binding,
                observer.as_ref(),
            )
            .await;
            let resolved_turn = resolve_provider_turn(
                config,
                runtime,
                session_id,
                user_input,
                &preparation,
                provider_turn_result,
                error_mode,
                binding,
                ingress,
                observer.as_ref(),
            )
            .await;

            apply_resolved_provider_turn(
                config,
                runtime,
                session_id,
                user_input,
                &preparation,
                &resolved_turn,
                binding,
                observer.as_ref(),
            )
            .await
            .map(|reply| (reply, false))
        }
        .await;

        match turn_result {
            Ok((reply, true)) => {
                observe_non_provider_turn_terminal_success_phases(observer.as_ref());
                Ok(reply)
            }
            Ok((reply, false)) => Ok(reply),
            Err(error) => {
                let failed_event = ConversationTurnPhaseEvent::failed();
                observe_turn_phase(observer.as_ref(), failed_event);
                Err(error)
            }
        }
    }

    #[cfg(feature = "memory-sqlite")]
    async fn maybe_handle_pending_approval_control_turn<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        runtime: &R,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> CliResult<Option<String>> {
        let Some(control_decision) = parse_pending_approval_input_decision(user_input) else {
            return Ok(None);
        };

        if let Some(kernel_ctx) = binding.kernel_context() {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }

        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config)?;
        let Some(pending_request) = repo
            .list_approval_requests_for_session(session_id, Some(ApprovalRequestStatus::Pending))?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };

        observe_turn_phase(observer, ConversationTurnPhaseEvent::preparing());
        let session_context = runtime.session_context(config, session_id, binding)?;
        let assembled_context = runtime
            .build_context(config, session_id, true, binding)
            .await?;
        let turn_id = next_conversation_turn_id();
        let preparation = ProviderTurnPreparation::from_assembled_context_with_turn_id(
            config,
            assembled_context,
            user_input,
            turn_id.as_str(),
            None,
        );
        observe_turn_phase(
            observer,
            ConversationTurnPhaseEvent::context_ready(
                preparation.session.messages.len(),
                preparation.session.estimated_tokens,
            ),
        );

        let mut approval_args = serde_json::Map::new();
        approval_args.insert(
            "approval_request_id".to_owned(),
            Value::String(pending_request.approval_request_id.clone()),
        );
        approval_args.insert(
            "decision".to_owned(),
            Value::String(control_decision.approval_decision().as_str().to_owned()),
        );
        if let Some(session_consent_mode) = control_decision.session_mode() {
            approval_args.insert(
                "session_consent_mode".to_owned(),
                Value::String(session_consent_mode.as_str().to_owned()),
            );
        }

        let approval_turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "approval_request_resolve".to_owned(),
                args_json: Value::Object(approval_args),
                source: "approval_control".to_owned(),
                session_id: session_context.session_id.clone(),
                turn_id: preparation.turn_id.clone(),
                tool_call_id: format!(
                    "call-approval-control-{}",
                    normalize_pending_approval_control_input(user_input)
                ),
            }],
            raw_meta: Value::Null,
        };

        let resolved_turn = resolve_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            Ok(approval_turn),
            error_mode,
            binding,
            None,
            observer,
        )
        .await;
        let reply = apply_resolved_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            &resolved_turn,
            binding,
            observer,
        )
        .await?;
        Ok(Some(reply.reply))
    }

    fn reload_followup_provider_config_after_tool_turn(
        config: &LoongClawConfig,
        turn: &ProviderTurn,
    ) -> LoongClawConfig {
        let config_path_from_tool = turn.tool_intents.iter().rev().find_map(|intent| {
            let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
            let payload = if canonical_tool_name == "provider.switch" {
                intent.args_json.as_object()
            } else if canonical_tool_name == "tool.invoke" {
                intent
                    .args_json
                    .as_object()
                    .filter(|payload| {
                        payload
                            .get("tool_id")
                            .and_then(Value::as_str)
                            .map(crate::tools::canonical_tool_name)
                            == Some("provider.switch")
                    })
                    .and_then(|payload| payload.get("arguments"))
                    .and_then(Value::as_object)
            } else {
                None
            };

            payload
                .and_then(|payload| payload.get("config_path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        });

        let config_path = config_path_from_tool.or_else(|| {
            crate::tools::runtime_config::get_tool_runtime_config()
                .config_path
                .clone()
        });
        let Some(config_path) = config_path else {
            return config.clone();
        };

        config
            .reload_provider_runtime_state_from_path(config_path.as_path())
            .unwrap_or_else(|_| config.clone())
    }

    pub async fn handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_options: &AcpConversationTurnOptions<'_>,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
    ) -> CliResult<String> {
        let observer_ref = observer.as_ref();
        let production_binding = require_production_kernel_binding(binding, observer_ref)?;

        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            production_binding,
            ingress,
            observer,
        )
        .await
    }

    pub(crate) async fn handle_turn_with_runtime_and_address_and_acp_event_sink<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        acp_event_sink: Option<&dyn AcpTurnEventSink>,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::from_event_sink(acp_event_sink);
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            binding,
            None,
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
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let session_id = address.session_id.as_str();
        let executed =
            execute_acp_conversation_turn_for_address(config, address, user_input, acp_options)
                .await?;
        let persistence_context = &executed.persistence_context;

        match executed.outcome {
            AcpConversationTurnExecutionOutcome::Succeeded(success) => {
                let reply = success.result.output_text.clone();
                persist_reply_turns_raw_with_mode(
                    runtime,
                    session_id,
                    user_input,
                    &reply,
                    ReplyPersistenceMode::Success,
                    binding,
                )
                .await?;
                if config.acp.emit_runtime_events {
                    let _ = persist_acp_runtime_events(
                        runtime,
                        session_id,
                        persistence_context,
                        &success.runtime_events,
                        Some(&success.result),
                        None,
                        binding,
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
                        binding,
                    )
                    .await;
                }
                match error_mode {
                    ProviderErrorMode::Propagate => Err(failure.error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&failure.error);
                        persist_reply_turns_raw_with_mode(
                            runtime,
                            session_id,
                            user_input,
                            &synthetic,
                            ReplyPersistenceMode::InlineProviderError,
                            binding,
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
    binding: ConversationRuntimeBinding<'_>,
    force: bool,
) -> CliResult<ContextCompactionOutcome> {
    let estimated_tokens = estimated_tokens.or_else(|| estimate_tokens(messages));
    let should_attempt_compaction = if force {
        true
    } else {
        config
            .conversation
            .should_compact_with_estimate(messages.len(), estimated_tokens)
    };
    if !should_attempt_compaction {
        return Ok(ContextCompactionOutcome::Skipped);
    }
    let Some(kernel_ctx) = binding.kernel_context() else {
        return Ok(ContextCompactionOutcome::Skipped);
    };

    #[cfg(feature = "memory-sqlite")]
    {
        if let Err(error) = persist_runtime_self_continuity_for_compaction(config, session_id) {
            if config.conversation.compaction_fail_open() {
                return Ok(ContextCompactionOutcome::FailedOpen);
            }

            return Err(format!(
                "pre-compaction runtime self continuity persist failed: {error}"
            ));
        }

        let workspace_root = config
            .tools
            .file_root
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|_| config.tools.resolved_file_root());

        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        let compact_stage_result =
            crate::memory::run_compact_stage(session_id, workspace_root.as_deref(), &memory_config)
                .await;
        match compact_stage_result {
            Ok(diagnostics)
                if matches!(diagnostics.outcome, crate::memory::StageOutcome::Fallback) =>
            {
                if config.conversation.compaction_fail_open() {
                    return Ok(ContextCompactionOutcome::FailedOpen);
                }

                return Err(format!(
                    "pre-compaction durable memory flush failed: {}",
                    diagnostics
                        .message
                        .as_deref()
                        .unwrap_or("compact stage fallback without error detail")
                ));
            }
            Ok(_) => {}
            Err(_error) if config.conversation.compaction_fail_open() => {
                return Ok(ContextCompactionOutcome::FailedOpen);
            }
            Err(error) => {
                return Err(format!(
                    "pre-compaction durable memory flush failed: {error}"
                ));
            }
        }
    }

    match runtime
        .compact_context(config, session_id, messages, kernel_ctx)
        .await
    {
        Ok(()) => Ok(ContextCompactionOutcome::Completed),
        Err(_error) if config.conversation.compaction_fail_open() => {
            Ok(ContextCompactionOutcome::FailedOpen)
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn persist_runtime_self_continuity_for_compaction(
    config: &LoongClawConfig,
    session_id: &str,
) -> Result<(), String> {
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;

    ensure_session_exists_for_runtime_self_continuity(&repo, session_id)?;

    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let stored_continuity =
        runtime_self_continuity::load_persisted_runtime_self_continuity(&repo, session_id)?;
    let continuity = runtime_self_continuity::merge_runtime_self_continuity(
        live_continuity,
        stored_continuity.as_ref(),
    );
    let Some(continuity) = continuity else {
        return Ok(());
    };
    if stored_continuity.as_ref() == Some(&continuity) {
        return Ok(());
    }

    let payload = json!({
        "source": "compaction",
        "runtime_self_continuity": continuity,
    });
    let event = NewSessionEvent {
        session_id: session_id.to_owned(),
        event_kind: runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND.to_owned(),
        actor_session_id: Some(session_id.to_owned()),
        payload_json: payload,
    };
    repo.append_event(event)?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn ensure_session_exists_for_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<(), String> {
    let existing_session = repo.load_session(session_id)?;
    if existing_session.is_some() {
        return Ok(());
    }

    let summary = repo.load_session_summary_with_legacy_fallback(session_id)?;
    let delegate_parent_session_id = repo
        .list_delegate_lifecycle_events(session_id)?
        .into_iter()
        .rev()
        .find_map(|event| event.actor_session_id);
    let kind = summary.as_ref().map(|value| value.kind).unwrap_or_else(|| {
        if session_id.starts_with("delegate:") || delegate_parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        }
    });
    let parent_session_id = match kind {
        SessionKind::Root => None,
        SessionKind::DelegateChild => {
            let stored_parent_session_id = summary
                .as_ref()
                .and_then(|value| value.parent_session_id.clone());
            let reconstructed_parent_session_id =
                delegate_parent_session_id.or(stored_parent_session_id);
            let Some(reconstructed_parent_session_id) = reconstructed_parent_session_id else {
                return Err(format!(
                    "delegate session `{session_id}` is missing lineage required for runtime self continuity persistence"
                ));
            };
            Some(reconstructed_parent_session_id)
        }
    };
    let label = summary.as_ref().and_then(|value| value.label.clone());
    let state = summary
        .as_ref()
        .map(|value| value.state)
        .unwrap_or(SessionState::Ready);
    let record = NewSessionRecord {
        session_id: session_id.to_owned(),
        kind,
        parent_session_id,
        label,
        state,
    };
    let _ = repo.ensure_session(record)?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn effective_runtime_self_continuity_for_session(
    config: &LoongClawConfig,
    session_context: &SessionContext,
) -> Option<runtime_self_continuity::RuntimeSelfContinuity> {
    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let stored_continuity = session_context.runtime_self_continuity.as_ref();
    runtime_self_continuity::merge_runtime_self_continuity(live_continuity, stored_continuity)
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

fn analytics_turn_checkpoint_progress_status(
    status: TurnCheckpointProgressStatus,
) -> AnalyticsTurnCheckpointProgressStatus {
    match status {
        TurnCheckpointProgressStatus::Pending => AnalyticsTurnCheckpointProgressStatus::Pending,
        TurnCheckpointProgressStatus::Skipped => AnalyticsTurnCheckpointProgressStatus::Skipped,
        TurnCheckpointProgressStatus::Completed => AnalyticsTurnCheckpointProgressStatus::Completed,
        TurnCheckpointProgressStatus::Failed => AnalyticsTurnCheckpointProgressStatus::Failed,
        TurnCheckpointProgressStatus::FailedOpen => {
            AnalyticsTurnCheckpointProgressStatus::FailedOpen
        }
    }
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

fn observe_turn_phase(
    observer: Option<&ConversationTurnObserverHandle>,
    event: ConversationTurnPhaseEvent,
) {
    let Some(observer) = observer else {
        return;
    };

    observer.on_phase(event);
}

fn observe_non_provider_turn_terminal_success_phases(
    observer: Option<&ConversationTurnObserverHandle>,
) {
    let finalizing_event = ConversationTurnPhaseEvent {
        phase: ConversationTurnPhase::FinalizingReply,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
    };
    observe_turn_phase(observer, finalizing_event);

    let completed_event = ConversationTurnPhaseEvent {
        phase: ConversationTurnPhase::Completed,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
    };
    observe_turn_phase(observer, completed_event);
}

fn observe_provider_turn_tool_batch_started(
    observer: Option<&ConversationTurnObserverHandle>,
    turn: &ProviderTurn,
) {
    let Some(observer) = observer else {
        return;
    };

    for intent in &turn.tool_intents {
        let tool_name = effective_result_tool_name(intent);
        let request_summary = summarize_single_tool_followup_request(intent);
        let event = ConversationTurnToolEvent::running(intent.tool_call_id.clone(), tool_name)
            .with_request_summary(request_summary);
        observer.on_tool(event);
    }
}

fn observe_provider_turn_tool_batch_terminal(
    observer: Option<&ConversationTurnObserverHandle>,
    tool_events: &[ConversationTurnToolEvent],
) {
    let Some(observer) = observer else {
        return;
    };

    for tool_event in tool_events {
        observer.on_tool(tool_event.clone());
    }
}

fn build_provider_turn_tool_terminal_events(
    turn: &ProviderTurn,
    turn_result: &TurnResult,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Vec<ConversationTurnToolEvent> {
    let mut trace_events = BTreeMap::new();
    if let Some(trace) = trace {
        for intent_outcome in &trace.intent_outcomes {
            let event = match intent_outcome.status {
                ToolBatchExecutionIntentStatus::Completed => ConversationTurnToolEvent::completed(
                    intent_outcome.tool_call_id.clone(),
                    intent_outcome.tool_name.clone(),
                    intent_outcome.detail.clone(),
                ),
                ToolBatchExecutionIntentStatus::NeedsApproval => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::needs_approval(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
                ToolBatchExecutionIntentStatus::Denied => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::denied(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
                ToolBatchExecutionIntentStatus::Failed => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::failed(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
            };
            trace_events.insert(intent_outcome.tool_call_id.clone(), event);
        }
    }

    let mut events = Vec::new();
    let mut unresolved_failure_emitted = false;

    for intent in &turn.tool_intents {
        if let Some(event) = trace_events.remove(intent.tool_call_id.as_str()) {
            let request_summary = summarize_single_tool_followup_request(intent);
            let event = event.with_request_summary(request_summary);
            events.push(event);
            continue;
        }

        let tool_name = effective_result_tool_name(intent);
        let fallback_event = match turn_result {
            TurnResult::FinalText(_)
            | TurnResult::StreamingText(_)
            | TurnResult::StreamingDone(_) => Some(ConversationTurnToolEvent::completed(
                intent.tool_call_id.clone(),
                tool_name,
                None,
            )),
            TurnResult::NeedsApproval(requirement) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::needs_approval(
                        intent.tool_call_id.clone(),
                        tool_name,
                        requirement.reason.clone(),
                    ))
                }
            }
            TurnResult::ToolDenied(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::denied(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
            TurnResult::ToolError(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::failed(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
            TurnResult::ProviderError(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::interrupted(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
        };

        if let Some(fallback_event) = fallback_event {
            let request_summary = summarize_single_tool_followup_request(intent);
            let fallback_event = fallback_event.with_request_summary(request_summary);
            events.push(fallback_event);
        }
    }

    events
}

#[cfg(test)]
fn summarize_tool_event_request(intent: &ToolIntent) -> Option<String> {
    summarize_single_tool_followup_request(intent)
}
fn provider_turn_observer_supports_streaming(
    config: &LoongClawConfig,
    observer: Option<&ConversationTurnObserverHandle>,
) -> bool {
    if observer.is_none() {
        return false;
    }

    crate::provider::supports_turn_streaming_events(config)
}

async fn request_provider_turn_with_observer<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    tool_view: &crate::tools::ToolView,
    binding: ConversationRuntimeBinding<'_>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ProviderTurn> {
    if let Some(observer) = observer
        && provider_turn_observer_supports_streaming(config, Some(observer))
    {
        let on_token = build_observer_streaming_token_callback(observer);
        return runtime
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
            )
            .await;
    }

    runtime
        .request_turn(config, session_id, turn_id, messages, tool_view, binding)
        .await
}

async fn resolve_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    result: CliResult<ProviderTurn>,
    error_mode: ProviderErrorMode,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> ResolvedProviderTurn {
    let turn_loop_policy = ProviderTurnLoopPolicy::from_config(config);
    let mut turn_loop_state = ProviderTurnLoopState::default();

    match decide_provider_turn_request_action(result, error_mode) {
        ProviderTurnRequestAction::Continue { turn } => {
            let turn =
                scope_provider_turn_tool_intents(turn, session_id, preparation.turn_id.as_str());
            if let Some(reply) =
                turn_loop_state.circuit_breaker_reply(&turn_loop_policy, turn.tool_intents.len())
            {
                return build_turn_loop_circuit_breaker_resolved_turn(
                    preparation,
                    user_input,
                    turn.tool_intents.len(),
                    reply,
                );
            }
            let continue_phase = prepare_provider_turn_continue_phase(
                config,
                runtime,
                session_id,
                preparation,
                turn,
                &turn_loop_policy,
                &mut turn_loop_state,
                binding,
                ingress,
                observer,
                1,
                false,
            )
            .await;
            continue_phase
                .resolve(
                    runtime,
                    session_id,
                    preparation,
                    user_input,
                    &turn_loop_policy,
                    &mut turn_loop_state,
                    config
                        .conversation
                        .turn_loop
                        .max_discovery_followup_rounds
                        .saturating_add(1)
                        .max(1),
                    binding,
                    observer,
                )
                .await
        }
        ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
            ProviderTurnRequestTerminalPhase::persist_inline_provider_error(reply)
                .resolve(preparation, user_input)
        }
        ProviderTurnRequestAction::ReturnError { error } => {
            ProviderTurnRequestTerminalPhase::return_error(error).resolve(preparation, user_input)
        }
    }
}

fn scope_provider_turn_tool_intents(
    mut turn: ProviderTurn,
    session_id: &str,
    turn_id: &str,
) -> ProviderTurn {
    for intent in &mut turn.tool_intents {
        if intent.source.starts_with("provider_") {
            // Provider-originated intents: runtime scope is authoritative.
            intent.session_id = session_id.to_owned();
            intent.turn_id = turn_id.to_owned();
        } else {
            // Non-provider intents: only fill in if missing.
            if intent.session_id.trim().is_empty() {
                intent.session_id = session_id.to_owned();
            }
            if intent.turn_id.trim().is_empty() {
                intent.turn_id = turn_id.to_owned();
            }
        }
    }
    turn
}

fn provider_turn_usage(turn: &ProviderTurn) -> Option<Value> {
    turn.raw_meta.get("usage").cloned()
}

fn build_turn_loop_circuit_breaker_resolved_turn(
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    tool_intents: usize,
    reply: String,
) -> ResolvedProviderTurn {
    let checkpoint = build_resolved_provider_checkpoint(
        preparation,
        user_input,
        Some(reply.as_str()),
        TurnCheckpointRequest::Continue { tool_intents },
        None,
        None,
        TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
    );
    ResolvedProviderTurn::persist_reply(reply, None, checkpoint)
}

async fn prepare_provider_turn_continue_phase<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: ProviderTurn,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    provider_round: usize,
    followup_chain_active: bool,
) -> ProviderTurnContinuePhase {
    let tool_intents = turn.tool_intents.len();
    let lane = preparation.lane_plan.decision.lane;
    if tool_intents > 0 {
        let running_tools_event =
            ConversationTurnPhaseEvent::running_tools(provider_round, lane, tool_intents);
        observe_turn_phase(observer, running_tools_event);
        observe_provider_turn_tool_batch_started(observer, &turn);
    }
    let lane_execution = execute_provider_turn_lane(
        config,
        runtime,
        session_id,
        preparation,
        &turn,
        binding,
        ingress,
        observer,
        followup_chain_active,
    )
    .await;
    let should_emit_binding_trust_event =
        !matches!(lane, ExecutionLane::Safe) || config.conversation.safe_lane_emit_runtime_events;
    if should_emit_binding_trust_event {
        emit_runtime_binding_trust_event_if_needed(
            runtime,
            session_id,
            &lane_execution.turn_result,
            binding,
        )
        .await;
    }
    observe_provider_turn_tool_batch_terminal(observer, &lane_execution.tool_events);
    let loop_verdict = turn_loop_state.observe_turn(turn_loop_policy, &turn);
    let followup_config =
        ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(config, &turn);
    ProviderTurnContinuePhase::new(
        tool_intents,
        lane_execution,
        loop_verdict,
        followup_config,
        ingress,
    )
}

async fn resolve_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    continue_phase: &ProviderTurnContinuePhase,
    user_input: &str,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    remaining_provider_rounds: usize,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> ResolvedProviderTurn {
    enum ReplyLoopDecision {
        FinalizeDirect(String),
        Followup {
            raw_reply: String,
            payload: ToolDrivenFollowupPayload,
            requires_completion_pass: bool,
            loop_warning_reason: Option<String>,
        },
        GuardFollowup {
            raw_reply: String,
            reason: String,
            latest_tool_payload: Option<ToolDrivenFollowupPayload>,
        },
    }

    let mut current_preparation = preparation.clone();
    let mut current_continue_phase = continue_phase.clone();
    let mut remaining_provider_rounds = remaining_provider_rounds.max(1);
    let mut provider_round_index = 0usize;

    loop {
        let current_provider_round = provider_round_index.saturating_add(1);
        if current_continue_phase.lane_execution.discovery_search_turn {
            emit_discovery_first_event(
                runtime,
                session_id,
                "discovery_first_search_round",
                json!({
                    "provider_round": current_provider_round,
                    "search_tool_calls": current_continue_phase
                        .lane_execution
                        .search_tool_intents,
                    "raw_tool_output_requested": current_continue_phase
                        .lane_execution
                        .raw_tool_output_requested,
                    "initial_estimated_tokens": estimate_tokens_for_messages(
                        current_preparation.session.estimated_tokens,
                        &current_preparation.session.messages,
                    ),
                }),
                binding,
            )
            .await;
        }

        let reply_decision = match current_continue_phase.reply_phase.decision() {
            ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => {
                let latest_tool_payload = tool_driven_followup_payload(
                    current_continue_phase.lane_execution.had_tool_intents,
                    &current_continue_phase.lane_execution.turn_result,
                );
                if let Some(reason) = current_continue_phase.hard_stop_reason() {
                    ReplyLoopDecision::GuardFollowup {
                        raw_reply: reply.clone(),
                        reason: reason.to_owned(),
                        latest_tool_payload,
                    }
                } else if current_continue_phase
                    .lane_execution
                    .supports_provider_turn_followup
                    && (!current_continue_phase
                        .lane_execution
                        .raw_tool_output_requested
                        || current_continue_phase.lane_execution.discovery_search_turn
                        || assistant_preface_signals_provider_turn_followup(
                            current_continue_phase
                                .lane_execution
                                .assistant_preface
                                .as_str(),
                        ))
                    && let Some(payload) = latest_tool_payload
                {
                    ReplyLoopDecision::Followup {
                        raw_reply: reply.clone(),
                        payload,
                        requires_completion_pass: false,
                        loop_warning_reason: current_continue_phase
                            .loop_warning_reason()
                            .map(ToOwned::to_owned),
                    }
                } else {
                    ReplyLoopDecision::FinalizeDirect(reply.clone())
                }
            }
            ToolDrivenReplyBaseDecision::RequireFollowup {
                raw_reply,
                payload: followup,
            } => {
                if let Some(reason) = current_continue_phase.hard_stop_reason() {
                    ReplyLoopDecision::GuardFollowup {
                        raw_reply: raw_reply.clone(),
                        reason: reason.to_owned(),
                        latest_tool_payload: Some(followup.clone()),
                    }
                } else {
                    ReplyLoopDecision::Followup {
                        raw_reply: raw_reply.clone(),
                        payload: followup.clone(),
                        requires_completion_pass: true,
                        loop_warning_reason: current_continue_phase
                            .loop_warning_reason()
                            .map(ToOwned::to_owned),
                    }
                }
            }
        };

        match reply_decision {
            ReplyLoopDecision::FinalizeDirect(reply) => {
                let checkpoint = current_continue_phase.checkpoint(preparation, user_input, &reply);
                return ResolvedProviderTurn::persist_reply(
                    reply,
                    current_continue_phase.lane_execution.provider_usage.clone(),
                    checkpoint,
                );
            }
            ReplyLoopDecision::Followup {
                raw_reply,
                payload: followup,
                requires_completion_pass,
                loop_warning_reason,
            } => {
                let follow_up_messages = build_turn_reply_followup_messages_with_warning(
                    &current_preparation.session.messages,
                    current_continue_phase
                        .lane_execution
                        .assistant_preface
                        .as_str(),
                    followup.clone(),
                    current_continue_phase
                        .lane_execution
                        .tool_request_summary
                        .as_deref(),
                    user_input,
                    loop_warning_reason.as_deref(),
                );
                if current_continue_phase
                    .lane_execution
                    .supports_provider_turn_followup
                    && remaining_provider_rounds > 1
                {
                    let next_provider_round = current_provider_round.saturating_add(1);
                    remaining_provider_rounds -= 1;
                    let initial_estimated_tokens = estimate_tokens_for_messages(
                        current_preparation.session.estimated_tokens,
                        &current_preparation.session.messages,
                    );
                    let followup_request_estimated_tokens = estimate_tokens(&follow_up_messages);
                    let followup_added_estimated_tokens = initial_estimated_tokens
                        .zip(followup_request_estimated_tokens)
                        .map(|(initial, followup)| followup.saturating_sub(initial));
                    let followup_preparation =
                        current_preparation.for_followup_messages(follow_up_messages);
                    let followup_tool_view = match runtime.tool_view(
                        &current_continue_phase.followup_config,
                        session_id,
                        binding,
                    ) {
                        Ok(tool_view) => tool_view,
                        Err(_error) => {
                            let checkpoint = current_continue_phase.checkpoint(
                                preparation,
                                user_input,
                                raw_reply.as_str(),
                            );
                            return ResolvedProviderTurn::persist_reply(
                                raw_reply,
                                current_continue_phase.lane_execution.provider_usage.clone(),
                                checkpoint,
                            );
                        }
                    };
                    let followup_message_count = followup_preparation.session.messages.len();
                    let followup_context_estimated_tokens =
                        followup_preparation.session.estimated_tokens;
                    let followup_request_event =
                        ConversationTurnPhaseEvent::requesting_followup_provider(
                            next_provider_round,
                            current_continue_phase.lane_execution.lane,
                            current_continue_phase.tool_intent_count(),
                            followup_message_count,
                            followup_context_estimated_tokens,
                        );
                    observe_turn_phase(observer, followup_request_event);
                    emit_prompt_frame_event(
                        runtime,
                        session_id,
                        next_provider_round,
                        "followup",
                        followup_preparation.session.prompt_frame_summary(),
                        binding,
                    )
                    .await;
                    if current_continue_phase.lane_execution.discovery_search_turn {
                        emit_discovery_first_event(
                            runtime,
                            session_id,
                            "discovery_first_followup_requested",
                            json!({
                                "provider_round": provider_round_index.saturating_add(1),
                                "raw_tool_output_requested": current_continue_phase
                                    .lane_execution
                                    .raw_tool_output_requested,
                                "initial_estimated_tokens": initial_estimated_tokens,
                                "followup_estimated_tokens": followup_request_estimated_tokens,
                                "followup_added_estimated_tokens": followup_added_estimated_tokens,
                            }),
                            binding,
                        )
                        .await;
                    }
                    match decide_provider_turn_request_action(
                        request_provider_turn_with_observer(
                            &current_continue_phase.followup_config,
                            runtime,
                            session_id,
                            followup_preparation.turn_id.as_str(),
                            &followup_preparation.session.messages,
                            &followup_tool_view,
                            binding,
                            observer,
                        )
                        .await,
                        ProviderErrorMode::Propagate,
                    ) {
                        ProviderTurnRequestAction::Continue { turn } => {
                            let turn = scope_provider_turn_tool_intents(
                                turn,
                                session_id,
                                followup_preparation.turn_id.as_str(),
                            );
                            let followup_result = summarize_discovery_first_followup_turn(&turn);
                            if current_continue_phase.lane_execution.discovery_search_turn {
                                emit_discovery_first_event(
                                    runtime,
                                    session_id,
                                    "discovery_first_followup_result",
                                    json!({
                                        "provider_round": provider_round_index.saturating_add(1),
                                        "outcome": followup_result.outcome,
                                        "followup_tool_name": followup_result.followup_tool_name,
                                        "followup_target_tool_id": followup_result.followup_target_tool_id,
                                        "resolved_to_tool_invoke": followup_result
                                            .resolved_to_tool_invoke,
                                        "raw_tool_output_requested": current_continue_phase
                                            .lane_execution
                                            .raw_tool_output_requested,
                                    }),
                                    binding,
                                )
                                .await;
                            }
                            if let Some(reply) = turn_loop_state
                                .circuit_breaker_reply(turn_loop_policy, turn.tool_intents.len())
                            {
                                return build_turn_loop_circuit_breaker_resolved_turn(
                                    preparation,
                                    user_input,
                                    turn.tool_intents.len(),
                                    reply,
                                );
                            }
                            current_continue_phase = prepare_provider_turn_continue_phase(
                                &current_continue_phase.followup_config,
                                runtime,
                                session_id,
                                &followup_preparation,
                                turn,
                                turn_loop_policy,
                                turn_loop_state,
                                binding,
                                ingress,
                                observer,
                                next_provider_round,
                                current_continue_phase
                                    .lane_execution
                                    .supports_provider_turn_followup,
                            )
                            .await;
                            current_preparation = followup_preparation;
                            provider_round_index = provider_round_index.saturating_add(1);
                            continue;
                        }
                        ProviderTurnRequestAction::FinalizeInlineProviderError {
                            reply: provider_error_text,
                        }
                        | ProviderTurnRequestAction::ReturnError {
                            error: provider_error_text,
                        } => {
                            if current_continue_phase.lane_execution.discovery_search_turn {
                                emit_discovery_first_event(
                                    runtime,
                                    session_id,
                                    "discovery_first_followup_result",
                                    json!({
                                        "provider_round": provider_round_index.saturating_add(1),
                                        "outcome": "provider_error",
                                        "followup_tool_name": Value::Null,
                                        "followup_target_tool_id": Value::Null,
                                        "resolved_to_tool_invoke": false,
                                        "raw_tool_output_requested": current_continue_phase
                                            .lane_execution
                                            .raw_tool_output_requested,
                                    }),
                                    binding,
                                )
                                .await;
                            }
                            emit_provider_failover_trust_event_if_needed(
                                config,
                                runtime,
                                session_id,
                                provider_error_text.as_str(),
                                binding,
                            )
                            .await;
                            let checkpoint = current_continue_phase.checkpoint(
                                preparation,
                                user_input,
                                raw_reply.as_str(),
                            );
                            return ResolvedProviderTurn::persist_reply(
                                raw_reply,
                                current_continue_phase.lane_execution.provider_usage.clone(),
                                checkpoint,
                            );
                        }
                    }
                }
                if requires_completion_pass {
                    let reply = request_completion_with_raw_fallback(
                        runtime,
                        &current_continue_phase.followup_config,
                        &follow_up_messages,
                        binding,
                        raw_reply.as_str(),
                    )
                    .await;
                    let checkpoint =
                        current_continue_phase.checkpoint(preparation, user_input, reply.as_str());
                    return ResolvedProviderTurn::persist_reply(reply, None, checkpoint);
                }

                let checkpoint =
                    current_continue_phase.checkpoint(preparation, user_input, raw_reply.as_str());
                return ResolvedProviderTurn::persist_reply(
                    raw_reply,
                    current_continue_phase.lane_execution.provider_usage.clone(),
                    checkpoint,
                );
            }
            ReplyLoopDecision::GuardFollowup {
                raw_reply,
                reason,
                latest_tool_payload,
            } => {
                let guard_messages = build_turn_reply_guard_messages(
                    &current_preparation.session.messages,
                    current_continue_phase
                        .lane_execution
                        .assistant_preface
                        .as_str(),
                    reason.as_str(),
                    latest_tool_payload.as_ref(),
                    user_input,
                );
                let reply = request_completion_with_raw_fallback(
                    runtime,
                    &current_continue_phase.followup_config,
                    &guard_messages,
                    binding,
                    raw_reply.as_str(),
                )
                .await;
                let checkpoint =
                    current_continue_phase.checkpoint(preparation, user_input, reply.as_str());
                return ResolvedProviderTurn::persist_reply(reply, None, checkpoint);
            }
        }
    }
}

#[cfg(test)]
fn build_turn_reply_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    user_input: &str,
) -> Vec<Value> {
    build_turn_reply_followup_messages_with_warning(
        base_messages,
        assistant_preface,
        followup,
        None,
        user_input,
        None,
    )
}

fn build_turn_reply_followup_messages_with_warning(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    tool_request_summary: Option<&str>,
    user_input: &str,
    loop_warning_reason: Option<&str>,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    messages.extend(build_tool_driven_followup_tail_with_request_summary(
        assistant_preface,
        &followup,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        |label, text| reduce_followup_payload_for_model(label, text).into_owned(),
    ));
    messages
}

fn build_turn_reply_guard_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    reason: &str,
    latest_tool_payload: Option<&ToolDrivenFollowupPayload>,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    messages.extend(build_tool_loop_guard_tail(
        assistant_preface,
        reason,
        user_input,
        latest_tool_payload.map(ToolDrivenFollowupPayload::message_context),
        |label, text| reduce_followup_payload_for_model(label.as_str(), text).into_owned(),
    ));
    messages
}

#[cfg(feature = "memory-sqlite")]
async fn repair_turn_checkpoint_tail_entry<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    entry: &super::session_history::TurnCheckpointLatestEntry,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<TurnCheckpointTailRepairOutcome> {
    let summary = &entry.summary;
    let (action, repair_plan, resume_input) = match load_turn_checkpoint_tail_runtime_eligibility(
        config, runtime, session_id, entry, binding,
    )
    .await?
    {
        TurnCheckpointTailRuntimeEligibility::NotNeeded { action, reason } => {
            return Ok(TurnCheckpointTailRepairOutcome::from_summary(
                TurnCheckpointTailRepairStatus::NotNeeded,
                action,
                Some(TurnCheckpointTailRepairSource::Summary),
                reason,
                summary,
            ));
        }
        TurnCheckpointTailRuntimeEligibility::Manual {
            action,
            reason,
            source,
        } => {
            return Ok(TurnCheckpointTailRepairOutcome::from_summary(
                TurnCheckpointTailRepairStatus::ManualRequired,
                action,
                Some(source),
                reason,
                summary,
            ));
        }
        TurnCheckpointTailRuntimeEligibility::Runnable {
            action,
            plan,
            resume_input,
        } => (action, plan, resume_input),
    };

    let mut after_turn_status =
        restore_analytics_turn_checkpoint_progress_status(repair_plan.after_turn_status());
    let mut compaction_status =
        restore_analytics_turn_checkpoint_progress_status(repair_plan.compaction_status());

    if repair_plan.should_run_after_turn() {
        let Some(kernel_ctx) = binding.kernel_context() else {
            after_turn_status = TurnCheckpointProgressStatus::Skipped;
            if repair_plan.should_run_compaction() {
                compaction_status = TurnCheckpointProgressStatus::Skipped;
            }
            persist_turn_checkpoint_event_value(
                runtime,
                session_id,
                &entry.checkpoint,
                TurnCheckpointStage::Finalized,
                TurnCheckpointFinalizationProgress {
                    after_turn: after_turn_status,
                    compaction: compaction_status,
                },
                None,
                binding,
            )
            .await?;
            return Ok(TurnCheckpointTailRepairOutcome::repaired(
                action,
                summary,
                after_turn_status,
                compaction_status,
            ));
        };
        match runtime
            .after_turn(
                session_id,
                resume_input.user_input(),
                resume_input.assistant_reply(),
                resume_input.messages(),
                kernel_ctx,
            )
            .await
        {
            Ok(()) => {
                after_turn_status = TurnCheckpointProgressStatus::Completed;
            }
            Err(error) => {
                persist_turn_checkpoint_event_value(
                    runtime,
                    session_id,
                    &entry.checkpoint,
                    TurnCheckpointStage::FinalizationFailed,
                    TurnCheckpointFinalizationProgress {
                        after_turn: TurnCheckpointProgressStatus::Failed,
                        compaction: if repair_plan.should_run_compaction() {
                            TurnCheckpointProgressStatus::Skipped
                        } else {
                            compaction_status
                        },
                    },
                    Some(TurnCheckpointFailure {
                        step: TurnCheckpointFailureStep::AfterTurn,
                        error: error.clone(),
                    }),
                    binding,
                )
                .await?;
                return Err(error);
            }
        }
    }

    if repair_plan.should_run_compaction() {
        match maybe_compact_context(
            config,
            runtime,
            session_id,
            resume_input.messages(),
            resume_input.estimated_tokens(),
            binding,
            false,
        )
        .await
        {
            Ok(outcome) => {
                compaction_status = outcome.checkpoint_status();
            }
            Err(error) => {
                persist_turn_checkpoint_event_value(
                    runtime,
                    session_id,
                    &entry.checkpoint,
                    TurnCheckpointStage::FinalizationFailed,
                    TurnCheckpointFinalizationProgress {
                        after_turn: after_turn_status,
                        compaction: TurnCheckpointProgressStatus::Failed,
                    },
                    Some(TurnCheckpointFailure {
                        step: TurnCheckpointFailureStep::Compaction,
                        error: error.clone(),
                    }),
                    binding,
                )
                .await?;
                return Err(error);
            }
        }
    }

    persist_turn_checkpoint_event_value(
        runtime,
        session_id,
        &entry.checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        binding,
    )
    .await?;

    Ok(TurnCheckpointTailRepairOutcome::repaired(
        action,
        summary,
        after_turn_status,
        compaction_status,
    ))
}

#[cfg(feature = "memory-sqlite")]
async fn probe_turn_checkpoint_tail_runtime_gate_entry<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    entry: &super::session_history::TurnCheckpointLatestEntry,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    match load_turn_checkpoint_tail_runtime_eligibility(config, runtime, session_id, entry, binding)
        .await?
    {
        TurnCheckpointTailRuntimeEligibility::Manual {
            action,
            reason,
            source: TurnCheckpointTailRepairSource::Runtime,
        } => Ok(Some(TurnCheckpointTailRepairRuntimeProbe::new(
            action,
            TurnCheckpointTailRepairSource::Runtime,
            reason,
        ))),
        TurnCheckpointTailRuntimeEligibility::NotNeeded { .. }
        | TurnCheckpointTailRuntimeEligibility::Manual { .. }
        | TurnCheckpointTailRuntimeEligibility::Runnable { .. } => Ok(None),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn load_turn_checkpoint_tail_runtime_eligibility<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    entry: &super::session_history::TurnCheckpointLatestEntry,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<TurnCheckpointTailRuntimeEligibility> {
    let summary = &entry.summary;
    let recovery = TurnCheckpointRecoveryAssessment::from_summary(summary);
    let action = recovery.action();
    if matches!(action, TurnCheckpointRecoveryAction::None) {
        return Ok(TurnCheckpointTailRuntimeEligibility::NotNeeded {
            action,
            reason: TurnCheckpointTailRepairReason::NotNeeded,
        });
    }
    if matches!(action, TurnCheckpointRecoveryAction::InspectManually) {
        return Ok(TurnCheckpointTailRuntimeEligibility::Manual {
            action,
            reason: recovery
                .reason()
                .unwrap_or(TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection),
            source: recovery.source(),
        });
    }

    let repair_plan = build_turn_checkpoint_repair_plan(summary);
    let assembled = match runtime
        .build_context(config, session_id, true, binding)
        .await
    {
        Ok(assembled) => assembled,
        Err(error) => {
            tracing::warn!(
                target: "loongclaw.conversation",
                session_id = %session_id,
                error = %error,
                "failed to assemble runtime context for turn checkpoint tail repair; degrading to manual inspection"
            );
            return Ok(TurnCheckpointTailRuntimeEligibility::Manual {
                action: TurnCheckpointRecoveryAction::InspectManually,
                reason: TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection,
                source: TurnCheckpointTailRepairSource::Runtime,
            });
        }
    };
    match TurnCheckpointRepairResumeInput::from_assembled_context(assembled, &entry.checkpoint) {
        Ok(resume_input) => Ok(TurnCheckpointTailRuntimeEligibility::Runnable {
            action,
            plan: repair_plan,
            resume_input,
        }),
        Err(reason) => Ok(TurnCheckpointTailRuntimeEligibility::Manual {
            action: TurnCheckpointRecoveryAction::InspectManually,
            reason,
            source: TurnCheckpointTailRepairSource::Runtime,
        }),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn probe_turn_checkpoint_tail_runtime_gate_entry_with_limit<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let Some(entry) =
        load_latest_turn_checkpoint_entry(session_id, limit, binding, &memory_config).await?
    else {
        return Ok(None);
    };
    probe_turn_checkpoint_tail_runtime_gate_entry(config, runtime, session_id, &entry, binding)
        .await
}

async fn finalize_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    tail_phase: &ProviderTurnReplyTailPhase,
    usage: Option<Value>,
    checkpoint: &TurnCheckpointSnapshot,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<ConversationTurnOutcome> {
    let Some(persistence_mode) = checkpoint.finalization.persistence_mode() else {
        return Ok(ConversationTurnOutcome {
            reply: tail_phase.reply().to_owned(),
            usage,
        });
    };
    persist_reply_turns_with_mode(
        runtime,
        session_id,
        user_input,
        tail_phase.reply(),
        persistence_mode,
        binding,
    )
    .await?;

    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::PostPersist,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        binding,
    )
    .await?;

    let after_turn_status = if checkpoint.finalization.runs_after_turn() {
        if let Some(kernel_ctx) = binding.kernel_context() {
            match runtime
                .after_turn(
                    session_id,
                    user_input,
                    tail_phase.reply(),
                    tail_phase.after_turn_messages(),
                    kernel_ctx,
                )
                .await
            {
                Ok(()) => TurnCheckpointProgressStatus::Completed,
                Err(error) => {
                    persist_turn_checkpoint_event(
                        runtime,
                        session_id,
                        checkpoint,
                        TurnCheckpointStage::FinalizationFailed,
                        TurnCheckpointFinalizationProgress {
                            after_turn: TurnCheckpointProgressStatus::Failed,
                            compaction: TurnCheckpointProgressStatus::Skipped,
                        },
                        Some(TurnCheckpointFailure {
                            step: TurnCheckpointFailureStep::AfterTurn,
                            error: error.clone(),
                        }),
                        binding,
                    )
                    .await?;
                    return Err(error);
                }
            }
        } else {
            TurnCheckpointProgressStatus::Skipped
        }
    } else {
        TurnCheckpointProgressStatus::Skipped
    };
    let compaction_status = if checkpoint.finalization.attempts_context_compaction() {
        match maybe_compact_context(
            config,
            runtime,
            session_id,
            tail_phase.after_turn_messages(),
            tail_phase.estimated_tokens(),
            binding,
            false,
        )
        .await
        {
            Ok(outcome) => outcome.checkpoint_status(),
            Err(error) => {
                persist_turn_checkpoint_event(
                    runtime,
                    session_id,
                    checkpoint,
                    TurnCheckpointStage::FinalizationFailed,
                    TurnCheckpointFinalizationProgress {
                        after_turn: after_turn_status,
                        compaction: TurnCheckpointProgressStatus::Failed,
                    },
                    Some(TurnCheckpointFailure {
                        step: TurnCheckpointFailureStep::Compaction,
                        error: error.clone(),
                    }),
                    binding,
                )
                .await?;
                return Err(error);
            }
        }
    } else {
        TurnCheckpointProgressStatus::Skipped
    };
    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        binding,
    )
    .await?;
    Ok(ConversationTurnOutcome {
        reply: tail_phase.reply().to_owned(),
        usage,
    })
}

async fn persist_resolved_provider_error_checkpoint<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        binding,
    )
    .await
}

async fn apply_resolved_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    resolved: &ResolvedProviderTurn,
    binding: ConversationRuntimeBinding<'_>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ConversationTurnOutcome> {
    if let Some(error_text) = resolved.provider_error_text() {
        emit_provider_failover_trust_event_if_needed(
            config, runtime, session_id, error_text, binding,
        )
        .await;
    }
    let terminal_phase = resolved.terminal_phase(&preparation.session);
    let completion_event = match &terminal_phase {
        ProviderTurnTerminalPhase::PersistReply(phase) => {
            let message_count = phase.tail_phase.after_turn_messages().len();
            let estimated_tokens = phase.tail_phase.estimated_tokens();
            let finalizing_event =
                ConversationTurnPhaseEvent::finalizing_reply(message_count, estimated_tokens);
            observe_turn_phase(observer, finalizing_event);
            Some(ConversationTurnPhaseEvent::completed(
                message_count,
                estimated_tokens,
            ))
        }
        ProviderTurnTerminalPhase::ReturnError(_) => None,
    };
    let apply_result = terminal_phase
        .apply(config, runtime, session_id, user_input, binding)
        .await;

    let completion_observation = match (completion_event, apply_result.is_ok()) {
        (Some(event), true) => Some(event),
        (Some(_), false) | (None, true) | (None, false) => None,
    };

    if let Some(event) = completion_observation {
        observe_turn_phase(observer, event);
    }

    apply_result
}

fn effective_tool_config_for_session(
    tool_config: &crate::config::ToolConfig,
    session_context: &SessionContext,
) -> crate::config::ToolConfig {
    let mut tool_config = tool_config.clone();
    if session_context.parent_session_id.is_some() {
        tool_config.sessions.visibility = crate::config::SessionVisibility::SelfOnly;
    }
    tool_config
}

struct CoordinatorAppToolDispatcher<'a, R: ?Sized> {
    config: &'a LoongClawConfig,
    runtime: &'a R,
    fallback: &'a DefaultAppToolDispatcher,
}

#[async_trait]
impl<R> AppToolDispatcher for CoordinatorAppToolDispatcher<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &super::autonomy_policy::AutonomyTurnBudgetState,
    ) -> Result<super::turn_engine::ToolPreflightOutcome, String> {
        self.fallback
            .preflight_tool_intent_with_binding(
                session_context,
                intent,
                descriptor,
                binding,
                budget_state,
            )
            .await
    }

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<super::turn_engine::ApprovalRequirement>, String> {
        self.fallback
            .maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: loongclaw_contracts::ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        self.fallback
            .preflight_tool_execution_with_binding(
                session_context,
                intent,
                request,
                descriptor,
                binding,
            )
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loongclaw_contracts::ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
            "approval_request_resolve" => {
                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, binding);
                    Err("approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                        .to_owned())
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let memory_config =
                        MemoryRuntimeConfig::from_memory_config(&self.config.memory);
                    let effective_tool_config =
                        effective_tool_config_for_session(&self.config.tools, session_context);
                    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
                        self.config,
                        self.runtime,
                        self.fallback,
                        binding,
                    );
                    crate::tools::approval::execute_approval_tool_with_runtime_support(
                        request,
                        &session_context.session_id,
                        &memory_config,
                        &effective_tool_config,
                        Some(&approval_runtime),
                    )
                    .await
                }
            }
            "delegate" => {
                execute_delegate_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    binding,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    binding,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(session_context, request, binding)
                    .await
            }
        }
    }

    async fn after_tool_execution(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        intent_sequence: usize,
        request: &loongclaw_contracts::ToolCoreRequest,
        outcome: &loongclaw_contracts::ToolCoreOutcome,
        binding: ConversationRuntimeBinding<'_>,
    ) {
        let tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());

        persist_tool_discovery_refresh_event_if_needed(
            self.runtime,
            &session_context.session_id,
            intent,
            intent_sequence,
            tool_name,
            outcome,
            binding,
        )
        .await;
    }
}

async fn persist_tool_discovery_refresh_event_if_needed<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    intent: &ToolIntent,
    intent_sequence: usize,
    tool_name: &str,
    outcome: &loongclaw_contracts::ToolCoreOutcome,
    binding: ConversationRuntimeBinding<'_>,
) {
    if tool_name != "tool.search" {
        return;
    }

    if outcome.status != "ok" {
        return;
    }

    let Some(discovery_state) = ToolDiscoveryState::from_tool_search_payload(&outcome.payload)
    else {
        return;
    };
    let Some(discovery_payload) =
        build_tool_discovery_refresh_event_payload(discovery_state, intent, intent_sequence)
    else {
        return;
    };
    let persist_result = persist_conversation_event(
        runtime,
        session_id,
        TOOL_DISCOVERY_REFRESHED_EVENT_NAME,
        discovery_payload,
        binding,
    )
    .await;

    if persist_result.is_ok() {
        return;
    }

    let Some(ctx) = binding.kernel_context() else {
        return;
    };

    let _ = ctx.kernel.record_audit_event(
        Some(ctx.agent_id()),
        AuditEventKind::PlaneInvoked {
            pack_id: ctx.pack_id().to_owned(),
            plane: ExecutionPlane::Runtime,
            tier: PlaneTier::Core,
            primary_adapter: "conversation.runtime".to_owned(),
            delegated_core_adapter: None,
            operation: "conversation.runtime.tool_discovery_persist_failed".to_owned(),
            required_capabilities: Vec::new(),
        },
    );
}

fn build_tool_discovery_refresh_event_payload(
    discovery_state: ToolDiscoveryState,
    intent: &ToolIntent,
    intent_sequence: usize,
) -> Option<Value> {
    let discovery_payload = serde_json::to_value(discovery_state).ok()?;
    let Value::Object(mut discovery_payload) = discovery_payload else {
        return None;
    };
    let turn_id = intent.turn_id.trim();
    let tool_call_id = intent.tool_call_id.trim();

    if !turn_id.is_empty() {
        discovery_payload.insert("turn_id".to_owned(), Value::String(turn_id.to_owned()));
    }

    if !tool_call_id.is_empty() {
        discovery_payload.insert(
            "tool_call_id".to_owned(),
            Value::String(tool_call_id.to_owned()),
        );
    }

    discovery_payload.insert("intent_sequence".to_owned(), json!(intent_sequence));

    Some(Value::Object(discovery_payload))
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&payload)?;
    let delegate_policy =
        crate::tools::delegate::resolve_delegate_policy(&delegate_request, &config.tools.delegate);
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_policy.label.clone();
    let subagent_identity =
        crate::tools::delegate::subagent_identity_for_delegate_request(&delegate_request);
    let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(&config.memory))?;
    let next_child_depth = next_delegate_child_depth_for_delegate(config, &repo, session_context)?;
    let runtime_self_continuity =
        effective_runtime_self_continuity_for_session(config, session_context);
    let workspace_root =
        prepare_delegate_workspace_root(config, &child_session_id, delegate_policy.isolation)?;
    let workspace_cleanup_owned_by_child =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let workspace_cleanup_owned_by_child_for_work =
        std::sync::Arc::clone(&workspace_cleanup_owned_by_child);
    super::delegate_support::with_prepared_subagent_spawn_cleanup_if_kernel_bound(
        runtime,
        &session_context.session_id,
        &child_session_id,
        binding,
        || async {
            let (_, execution) = repo.create_delegate_child_session_with_event_if_within_limit(
                &session_context.session_id,
                config.tools.delegate.max_active_children,
                |active_children| {
                    let execution_policy = DelegateChildExecutionPolicy {
                        isolation: delegate_policy.isolation,
                        profile: delegate_policy.profile,
                        owner_kind: None,
                        timeout_seconds: delegate_policy.timeout_seconds,
                        allow_shell_in_child: delegate_policy.allow_shell_in_child,
                        child_tool_allowlist: delegate_policy.child_tool_allowlist.clone(),
                        runtime_narrowing: delegate_policy.runtime_narrowing.clone(),
                        workspace_root: workspace_root.clone(),
                    };
                    let seed = build_delegate_child_lifecycle_seed(
                        config,
                        binding,
                        ConstrainedSubagentMode::Inline,
                        next_child_depth,
                        active_children,
                        &session_context.session_id,
                        &child_session_id,
                        child_label.clone(),
                        &delegate_request.task,
                        runtime_self_continuity.as_ref(),
                        subagent_identity.clone(),
                        execution_policy,
                    );
                    Ok((seed.request, seed.execution))
                },
            )?;
            let outcome = run_started_delegate_child_turn_with_runtime(
                config,
                runtime,
                &child_session_id,
                &session_context.session_id,
                child_label,
                &delegate_request.task,
                delegate_policy.profile,
                execution,
                delegate_policy.timeout_seconds,
                binding,
            )
            .await?;
            workspace_cleanup_owned_by_child_for_work
                .store(true, std::sync::atomic::Ordering::Release);

            Ok(outcome)
        },
    )
    .await
    .inspect_err(|_error| {
        let cleanup_owned_by_child =
            workspace_cleanup_owned_by_child.load(std::sync::atomic::Ordering::Acquire);
        if cleanup_owned_by_child {
            return;
        }

        let _ = cleanup_prepared_delegate_workspace_root(
            delegate_policy.isolation,
            workspace_root.as_deref(),
        );
    })
}

#[cfg(feature = "memory-sqlite")]
async fn enqueue_delegate_async_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let runtime_handle = Handle::try_current()
        .map_err(|error| format!("delegate_async_runtime_unavailable: {error}"))?;
    let spawner = runtime
        .async_delegate_spawner(config)
        .ok_or_else(|| "delegate_async_not_configured".to_owned())?;
    let delegate_request = build_delegate_async_enqueue_request(
        config,
        runtime,
        session_context,
        delegate_request,
        ConstrainedSubagentOwnerKind::AsyncDelegateSpawner,
        binding,
    )
    .await?;
    let detached_config = std::sync::Arc::new(config.clone());
    spawn_async_delegate_detached(
        runtime_handle,
        detached_config,
        delegate_request.memory_config,
        spawner,
        delegate_request.request,
        config.tools.delegate.max_frozen_bytes,
        DelegateAnnounceSettings::from_config(config),
    );

    Ok(delegate_request.outcome)
}

#[cfg(feature = "memory-sqlite")]
async fn enqueue_background_task_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let spawner = runtime
        .background_task_spawner(config)
        .ok_or_else(|| {
            "background_task_host_unavailable: current runtime does not provide a durable async background-task host"
                .to_owned()
        })?;
    let delegate_request = build_delegate_async_enqueue_request(
        config,
        runtime,
        session_context,
        delegate_request,
        ConstrainedSubagentOwnerKind::BackgroundTaskHost,
        binding,
    )
    .await?;
    let spawn_result = spawner.spawn(delegate_request.request.clone()).await;

    if let Err(error) = spawn_result {
        finalize_async_delegate_spawn_failure_with_recovery(
            &delegate_request.memory_config,
            &delegate_request.request.child_session_id,
            &delegate_request.request.parent_session_id,
            delegate_request.request.label.clone(),
            delegate_request.request.profile,
            &delegate_request.request.execution,
            config.tools.delegate.max_frozen_bytes,
            error.clone(),
        )?;
        enqueue_delegate_result_announce_with_memory_config(
            delegate_request.memory_config.clone(),
            delegate_request.request.parent_session_id.clone(),
            delegate_request.request.child_session_id.clone(),
            DelegateAnnounceSettings::from_config(config),
        );
        emit_async_delegate_child_terminal_event(
            runtime,
            &delegate_request.request.parent_session_id,
            &delegate_request.request.child_session_id,
            delegate_request.request.label.as_deref(),
            delegate_request.request.profile,
            "failed",
            delegate_request.request.execution.isolation,
            0,
            None,
            Some(error.as_str()),
            None,
            delegate_request.request.execution.workspace_root.as_deref(),
            None,
            binding,
        )
        .await;
        return Err(error);
    }

    Ok(delegate_request.outcome)
}

#[cfg(feature = "memory-sqlite")]
struct PreparedAsyncDelegateEnqueue {
    memory_config: MemoryRuntimeConfig,
    request: AsyncDelegateSpawnRequest,
    outcome: loongclaw_contracts::ToolCoreOutcome,
}

#[cfg(feature = "memory-sqlite")]
async fn build_delegate_async_enqueue_request<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    owner_kind: ConstrainedSubagentOwnerKind,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<PreparedAsyncDelegateEnqueue, String> {
    let delegate_policy =
        crate::tools::delegate::resolve_delegate_policy(&delegate_request, &config.tools.delegate);
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_policy.label.clone();
    let subagent_identity =
        crate::tools::delegate::subagent_identity_for_delegate_request(&delegate_request);
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;

    ensure_session_exists_for_runtime_self_continuity(&repo, &session_context.session_id)?;

    let next_child_depth = next_delegate_child_depth_for_delegate(config, &repo, session_context)?;
    let runtime_self_continuity =
        effective_runtime_self_continuity_for_session(config, session_context);
    let workspace_root =
        prepare_delegate_workspace_root(config, &child_session_id, delegate_policy.isolation)?;
    let (_, execution) = repo
        .create_delegate_child_session_with_event_if_within_limit(
            &session_context.session_id,
            config.tools.delegate.max_active_children,
            |active_children| {
                let execution_policy = DelegateChildExecutionPolicy {
                    isolation: delegate_policy.isolation,
                    profile: delegate_policy.profile,
                    owner_kind: Some(owner_kind),
                    timeout_seconds: delegate_policy.timeout_seconds,
                    allow_shell_in_child: delegate_policy.allow_shell_in_child,
                    child_tool_allowlist: delegate_policy.child_tool_allowlist.clone(),
                    runtime_narrowing: delegate_policy.runtime_narrowing.clone(),
                    workspace_root: workspace_root.clone(),
                };
                let seed = build_delegate_child_lifecycle_seed(
                    config,
                    binding,
                    ConstrainedSubagentMode::Async,
                    next_child_depth,
                    active_children,
                    &session_context.session_id,
                    &child_session_id,
                    child_label.clone(),
                    &delegate_request.task,
                    runtime_self_continuity.as_ref(),
                    subagent_identity.clone(),
                    execution_policy,
                );
                Ok((seed.request, seed.execution))
            },
        )
        .inspect_err(|_error| {
            let _ = cleanup_prepared_delegate_workspace_root(
                delegate_policy.isolation,
                workspace_root.as_deref(),
            );
        })?;

    let queued_execution = execution.clone();
    let queued_workspace_root = execution.workspace_root.clone();
    emit_async_delegate_child_queued_event(
        runtime,
        &session_context.session_id,
        &child_session_id,
        delegate_policy.label.as_deref(),
        delegate_policy.profile,
        delegate_policy.isolation,
        delegate_policy.timeout_seconds,
        queued_workspace_root.as_deref(),
        binding,
    )
    .await;
    let request = AsyncDelegateSpawnRequest {
        child_session_id: child_session_id.clone(),
        parent_session_id: session_context.session_id.clone(),
        task: delegate_request.task,
        label: child_label,
        profile: delegate_policy.profile,
        execution: queued_execution,
        runtime_self_continuity,
        timeout_seconds: delegate_policy.timeout_seconds,
        binding: OwnedConversationRuntimeBinding::from_borrowed(binding),
    };

    let mut outcome = crate::tools::delegate::delegate_async_queued_outcome(
        child_session_id,
        Some(session_context.session_id.clone()),
        delegate_policy.label,
        delegate_policy.profile,
        delegate_policy.timeout_seconds,
    );
    inject_delegate_workspace_metadata(&mut outcome, &execution, None, None);

    Ok(PreparedAsyncDelegateEnqueue {
        memory_config,
        request,
        outcome,
    })
}

#[cfg(feature = "memory-sqlite")]
pub async fn spawn_background_delegate_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    task: &str,
    label: Option<String>,
    _specialization: Option<String>,
    timeout_seconds: Option<u64>,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    let session_context = runtime.session_context(config, session_id, binding)?;
    let mut delegate_payload = json!({
        "task": task,
    });
    if let Some(label) = label
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("label".to_owned(), json!(label));
    }
    if let Some(specialization) = _specialization
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("specialization".to_owned(), json!(specialization));
    }
    if let Some(timeout_seconds) = timeout_seconds
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("timeout_seconds".to_owned(), json!(timeout_seconds));
    }
    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&delegate_payload)?;
    enqueue_background_task_with_runtime(
        config,
        runtime,
        &session_context,
        delegate_request,
        binding,
    )
    .await
}

#[cfg(not(feature = "memory-sqlite"))]
pub async fn spawn_background_delegate_with_runtime<R: ConversationRuntime + ?Sized>(
    _config: &LoongClawConfig,
    _runtime: &R,
    _session_id: &str,
    _task: &str,
    _label: Option<String>,
    _specialization: Option<String>,
    _timeout_seconds: Option<u64>,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    Err("delegate_async requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !binding.allows_mutation() {
        return Err("app_tool_denied: governed_runtime_binding_required".to_owned());
    }

    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&payload)?;
    enqueue_delegate_async_with_runtime(config, runtime, session_context, delegate_request, binding)
        .await
}

#[cfg(not(feature = "memory-sqlite"))]
pub(super) async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongClawConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    Err("delegate requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(not(feature = "memory-sqlite"))]
pub(super) async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongClawConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    Err("delegate_async requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn run_started_delegate_child_turn_with_runtime<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongClawConfig,
    runtime: &R,
    child_session_id: &str,
    parent_session_id: &str,
    child_label: Option<String>,
    user_input: &str,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    execution: ConstrainedSubagentExecution,
    timeout_seconds: u64,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    // This resumes execution after the child session row/workspace have already
    // been created. The remaining job is to run the child turn with the shared
    // runtime/binding, enforce timeout + unwind containment, and then finalize
    // the persisted delegate session/announcement state from the outcome.
    let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(&config.memory))?;
    let start = Instant::now();
    let child_coordinator = ConversationTurnCoordinator::new();
    let child_turn_future = child_coordinator.handle_turn_with_runtime(
        config,
        child_session_id,
        user_input,
        ProviderErrorMode::Propagate,
        runtime,
        binding,
    );
    // Heap-allocate nested delegate turn futures so each child turn does not inline the next
    // delegate layer into the parent future on small-stack platforms like Windows.
    let child_turn_future = Box::pin(AssertUnwindSafe(child_turn_future).catch_unwind());
    let child_timeout = Duration::from_secs(timeout_seconds);
    let child_result = timeout(child_timeout, child_turn_future).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match child_result {
        Ok(Ok(Ok(final_output))) => {
            let turn_count = repo
                .load_session_summary(child_session_id)?
                .map(|session| session.turn_count)
                .unwrap_or_default();
            let final_output_preview = final_output.clone();
            let mut outcome = crate::tools::delegate::delegate_success_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                final_output,
                turn_count,
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Completed,
                duration_ms,
                Some(turn_count),
                None,
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Completed,
                None,
                "delegate_completed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "completed",
                    execution.isolation,
                    duration_ms,
                    Some(turn_count),
                    None,
                    Some(final_output_preview.as_str()),
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            Ok(outcome)
        }
        Ok(Ok(Err(error))) => {
            let mut outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                error.clone(),
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Failed,
                duration_ms,
                None,
                Some(error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Failed,
                Some(error.clone()),
                "delegate_failed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "failed",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            Ok(outcome)
        }
        Ok(Err(panic_payload)) => {
            let panic_error = format_delegate_child_panic(panic_payload);
            let mut outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                panic_error.clone(),
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Failed,
                duration_ms,
                None,
                Some(panic_error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Failed,
                Some(panic_error.clone()),
                "delegate_failed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "failed",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(panic_error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            Ok(outcome)
        }
        Err(_) => {
            let timeout_error = "delegate_timeout".to_owned();
            let mut outcome = crate::tools::delegate::delegate_timeout_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::TimedOut,
                duration_ms,
                None,
                Some(timeout_error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::TimedOut,
                Some(timeout_error.clone()),
                "delegate_timed_out",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "timed_out",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(timeout_error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            Ok(outcome)
        }
    }
}

async fn execute_provider_turn_lane<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: &ProviderTurn,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    followup_chain_active: bool,
) -> ProviderTurnLaneExecution {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let search_tool_intents = turn
        .tool_intents
        .iter()
        .filter(|intent| effective_followup_tool_name(intent) == "tool.search")
        .count();
    let discovery_search_turn = search_tool_intents > 0;
    let supports_provider_turn_followup = followup_chain_active || discovery_search_turn;
    let assistant_preface = turn.assistant_text.clone();
    let lane = preparation.lane_plan.decision.lane;
    let session_context = match runtime.session_context(config, session_id, binding) {
        Ok(session_context) => session_context,
        Err(error) => {
            let turn_result = TurnResult::non_retryable_tool_error("session_context_failed", error);
            let tool_events = build_provider_turn_tool_terminal_events(turn, &turn_result, None);
            let tool_request_summary =
                summarize_provider_lane_tool_request(turn, &turn_result, None);
            return ProviderTurnLaneExecution {
                lane,
                assistant_preface,
                provider_usage: provider_turn_usage(turn),
                had_tool_intents,
                tool_request_summary,
                discovery_search_turn,
                search_tool_intents,
                supports_provider_turn_followup,
                raw_tool_output_requested: preparation.raw_tool_output_requested,
                turn_result,
                safe_lane_terminal_route: None,
                tool_events,
            };
        }
    };
    let base_app_dispatcher = DefaultAppToolDispatcher::with_config(
        MemoryRuntimeConfig::from_memory_config(&config.memory),
        config.clone(),
    );
    let app_dispatcher = CoordinatorAppToolDispatcher {
        config,
        runtime,
        fallback: &base_app_dispatcher,
    };
    let payload_summary_limit_chars = config
        .conversation
        .tool_result_payload_summary_limit_chars();
    let parallel_tool_execution_enabled = matches!(lane, ExecutionLane::Fast)
        && config
            .conversation
            .fast_lane_parallel_tool_execution_enabled;
    let parallel_tool_execution_max_in_flight = if parallel_tool_execution_enabled {
        config
            .conversation
            .fast_lane_parallel_tool_execution_max_in_flight()
    } else {
        1
    };
    let use_safe_lane_plan_path = preparation
        .lane_plan
        .should_use_safe_lane_plan_path(config, turn);
    let engine = TurnEngine::with_parallel_tool_execution(
        preparation.lane_plan.max_tool_steps,
        payload_summary_limit_chars,
        parallel_tool_execution_enabled,
        parallel_tool_execution_max_in_flight,
    );
    let validation = if use_safe_lane_plan_path {
        TurnEngine::with_tool_result_payload_summary_limit(usize::MAX, payload_summary_limit_chars)
            .validate_turn_in_context(turn, &session_context)
    } else {
        engine.validate_turn_in_context(turn, &session_context)
    };
    let (turn_result, safe_lane_terminal_route, fast_lane_tool_batch_trace) = match validation {
        Ok(TurnValidation::FinalText(text)) => (TurnResult::FinalText(text), None, None),
        Err(failure) => (TurnResult::ToolDenied(failure), None, None),
        Ok(TurnValidation::ToolExecutionRequired) if use_safe_lane_plan_path => {
            let outcome = execute_turn_with_safe_lane_plan(
                config,
                runtime,
                session_id,
                &preparation.lane_plan.decision,
                turn,
                &session_context,
                &app_dispatcher,
                binding,
                ingress,
            )
            .await;
            (outcome.result, outcome.terminal_route, None)
        }
        Ok(TurnValidation::ToolExecutionRequired) => {
            let (result, trace) = engine
                .execute_turn_in_context_with_trace(
                    turn,
                    &session_context,
                    &app_dispatcher,
                    binding,
                    ingress,
                    observer,
                )
                .await;
            (result, None, trace)
        }
    };

    if let Some(trace) = fast_lane_tool_batch_trace.as_ref() {
        let trace_persist_failed =
            persist_fast_lane_tool_trace(runtime, session_id, trace, binding)
                .await
                .is_err();
        if trace_persist_failed && let Some(ctx) = binding.kernel_context() {
            let _ = ctx.kernel.record_audit_event(
                Some(ctx.agent_id()),
                AuditEventKind::PlaneInvoked {
                    pack_id: ctx.pack_id().to_owned(),
                    plane: ExecutionPlane::Runtime,
                    tier: PlaneTier::Core,
                    primary_adapter: "conversation.fast_lane".to_owned(),
                    delegated_core_adapter: None,
                    operation: "conversation.fast_lane.tool_trace_persist_failed".to_owned(),
                    required_capabilities: Vec::new(),
                },
            );
        }

        let should_emit_batch_event = trace.has_execution_segments();
        let batch_event_failed = should_emit_batch_event
            && emit_fast_lane_tool_batch_event(runtime, session_id, trace, binding)
                .await
                .is_err();
        if batch_event_failed && let Some(ctx) = binding.kernel_context() {
            let _ = ctx.kernel.record_audit_event(
                Some(ctx.agent_id()),
                AuditEventKind::PlaneInvoked {
                    pack_id: ctx.pack_id().to_owned(),
                    plane: ExecutionPlane::Runtime,
                    tier: PlaneTier::Core,
                    primary_adapter: "conversation.fast_lane".to_owned(),
                    delegated_core_adapter: None,
                    operation: "conversation.fast_lane.fast_lane_tool_batch_persist_failed"
                        .to_owned(),
                    required_capabilities: Vec::new(),
                },
            );
        }
    }

    let tool_events = build_provider_turn_tool_terminal_events(
        turn,
        &turn_result,
        fast_lane_tool_batch_trace.as_ref(),
    );
    let tool_request_summary = summarize_provider_lane_tool_request(
        turn,
        &turn_result,
        fast_lane_tool_batch_trace.as_ref(),
    );
    let recovery_followup_turn = tool_driven_followup_payload(had_tool_intents, &turn_result)
        .is_some_and(|payload| {
            matches!(payload, ToolDrivenFollowupPayload::DiscoveryRecovery { .. })
        });
    let preface_signals_provider_turn_followup =
        assistant_preface_signals_provider_turn_followup(assistant_preface.as_str());
    let supports_provider_turn_followup = followup_chain_active
        || discovery_search_turn
        || recovery_followup_turn
        || preface_signals_provider_turn_followup;
    ProviderTurnLaneExecution {
        lane,
        assistant_preface,
        provider_usage: provider_turn_usage(turn),
        had_tool_intents,
        tool_request_summary,
        discovery_search_turn,
        search_tool_intents,
        supports_provider_turn_followup,
        raw_tool_output_requested: preparation.raw_tool_output_requested,
        turn_result,
        safe_lane_terminal_route,
        tool_events,
    }
}

fn assistant_preface_signals_provider_turn_followup(assistant_preface: &str) -> bool {
    let normalized_preface = assistant_preface.to_ascii_lowercase();
    let contains_first = normalized_preface.contains("first");
    let contains_then = normalized_preface.contains("then");
    let contains_next = normalized_preface.contains("next");
    let contains_after_that = normalized_preface.contains("after that");
    let contains_afterwards = normalized_preface.contains("afterwards");

    contains_first || contains_then || contains_next || contains_after_that || contains_afterwards
}
async fn execute_turn_with_safe_lane_plan<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
) -> SafeLaneTurnOutcome {
    let governor_history_signals =
        load_safe_lane_history_signals_for_governor(config, session_id, binding).await;
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
        binding,
    )
    .await;

    let mut state = SafeLanePlanLoopState::new(config, governor);

    loop {
        if let Some(min_anchor_matches) = state.refresh_verify_policy(config) {
            emit_safe_lane_event(
                config,
                runtime,
                session_id,
                "verify_policy_adjusted",
                json!({
                    "round": state.round(),
                    "policy": "adaptive_anchor_escalation",
                    "min_anchor_matches": min_anchor_matches,
                    "verify_failures": state.metrics.verify_failures,
                    "escalation_after_failures": config
                        .conversation
                        .safe_lane_verify_anchor_escalation_after_failures(),
                    "metrics": state.metrics.as_json(),
                }),
                binding,
            )
            .await;
        }

        state.note_round_started();
        emit_safe_lane_event(
            config,
            runtime,
            session_id,
            "plan_round_started",
            json!({
                "round": state.round(),
                "start_tool_index": state.plan_start_tool_index,
                "tool_node_max_attempts": state.tool_node_max_attempts(),
                "effective_max_rounds": state.max_rounds(),
                "effective_max_node_attempts": state.max_node_attempts(),
                "verify_min_anchor_matches": state.adaptive_verify_policy.min_anchor_matches,
                "session_governor": state.governor.as_json(),
                "metrics": state.metrics.as_json(),
            }),
            binding,
        )
        .await;

        let round_execution = evaluate_safe_lane_round(
            config,
            lane_decision,
            turn,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            &state,
        )
        .await;
        state.record_round_execution(&round_execution.report, round_execution.tool_output_stats);

        match round_execution.report.status.clone() {
            PlanRunStatus::Succeeded => {
                state.note_round_succeeded();
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": state.round(),
                        "status": "succeeded",
                        "attempts_used": round_execution.report.attempts_used,
                        "elapsed_ms": round_execution.report.elapsed_ms,
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;
                let tool_output = round_execution.tool_outputs.join("\n");
                let verify_report = verify_safe_lane_final_output(
                    config,
                    tool_output.as_str(),
                    turn.tool_intents.as_slice(),
                    state.adaptive_verify_policy,
                );
                if verify_report.passed {
                    emit_safe_lane_event(
                        config,
                        runtime,
                        session_id,
                        "final_status",
                        json!({
                            "status": "succeeded",
                            "round": state.round(),
                            "tool_output_stats": round_execution.tool_output_stats.as_json(),
                            "health_signal": derive_safe_lane_runtime_health_signal(
                                config,
                                state.metrics,
                                false,
                                None,
                            )
                            .as_json(),
                            "metrics": state.metrics.as_json(),
                        }),
                        binding,
                    )
                    .await;
                    return SafeLaneTurnOutcome::without_terminal_route(TurnResult::FinalText(
                        tool_output,
                    ));
                }

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
                state.note_verify_failure();
                let verify_route = decide_safe_lane_failure_route(
                    config,
                    &verify_failure,
                    state.replan_budget,
                    state.metrics,
                    &state.governor,
                );
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "verify_failed",
                    json!({
                        "round": state.round(),
                        "error": verify_error.clone(),
                        "failure_codes": failure_codes,
                        "retryable": retryable_verify_failure,
                        "failure_kind": format_turn_failure_kind(verify_failure.kind),
                        "failure_code": verify_failure.code.clone(),
                        "failure_retryable": verify_failure.retryable,
                        "route_decision": verify_route.decision_label(),
                        "route_reason": verify_route.reason.as_str(),
                        "route_source": verify_route.source_label(),
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;

                match decide_safe_lane_verify_failure_action(
                    verify_error.as_str(),
                    retryable_verify_failure,
                    verify_route,
                ) {
                    SafeLaneRoundDecision::Finalize { result } => {
                        let failure_meta = result.failure();
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "final_status",
                            json!({
                                "status": "failed",
                                "round": state.round(),
                                "failure": verify_route.verify_terminal_summary_label(),
                                "failure_kind": failure_meta
                                    .map(|failure| format_turn_failure_kind(failure.kind)),
                                "failure_code": failure_meta.map(|failure| failure.code.clone()),
                                "failure_retryable": failure_meta.map(|failure| failure.retryable),
                                "route_decision": verify_route.decision_label(),
                                "route_reason": verify_route.reason.as_str(),
                                "route_source": verify_route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    true,
                                    failure_meta.map(|failure| failure.code.as_str()),
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                        return SafeLaneTurnOutcome::with_terminal_route(result, verify_route);
                    }
                    SafeLaneRoundDecision::Replan {
                        reason,
                        next_plan_start_tool_index,
                        next_seed_tool_outputs,
                    } => {
                        state.note_replan(next_plan_start_tool_index, next_seed_tool_outputs);
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "replan_triggered",
                            json!({
                                "round": state.round(),
                                "reason": reason,
                                "detail": verify_error,
                                "route_decision": verify_route.decision_label(),
                                "route_reason": verify_route.reason.as_str(),
                                "route_source": verify_route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                    }
                }
            }
            PlanRunStatus::Failed(failure) => {
                state.note_round_failed();
                let round_failure_meta = turn_failure_from_plan_failure(&failure);
                let route = decide_safe_lane_failure_route(
                    config,
                    &round_failure_meta,
                    state.replan_budget,
                    state.metrics,
                    &state.governor,
                );
                let failure_summary = summarize_plan_failure(&failure);
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": state.round(),
                        "status": "failed",
                        "attempts_used": round_execution.report.attempts_used,
                        "elapsed_ms": round_execution.report.elapsed_ms,
                        "failure": failure_summary.clone(),
                        "failure_kind": format_turn_failure_kind(round_failure_meta.kind),
                        "failure_code": round_failure_meta.code.clone(),
                        "failure_retryable": round_failure_meta.retryable,
                        "route_decision": route.decision_label(),
                        "route_reason": route.reason.as_str(),
                        "route_source": route.source_label(),
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;
                let (next_start_tool_index, next_seed_outputs) = if route.should_replan() {
                    let (next_start_tool_index, next_seed_outputs) = derive_replan_cursor(
                        &failure,
                        round_execution.tool_outputs.as_slice(),
                        turn.tool_intents.len(),
                    );
                    (next_start_tool_index, next_seed_outputs)
                } else {
                    (0, Vec::new())
                };
                match decide_safe_lane_plan_failure_action(
                    failure.clone(),
                    route,
                    next_start_tool_index,
                    next_seed_outputs,
                ) {
                    SafeLaneRoundDecision::Finalize { result } => {
                        let failure_meta = result.failure();
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "final_status",
                            json!({
                                "status": "failed",
                                "round": state.round(),
                                "failure": failure_summary,
                                "failure_kind": failure_meta
                                    .map(|failure| format_turn_failure_kind(failure.kind)),
                                "failure_code": failure_meta.map(|failure| failure.code.clone()),
                                "failure_retryable": failure_meta.map(|failure| failure.retryable),
                                "route_decision": route.decision_label(),
                                "route_reason": route.reason.as_str(),
                                "route_source": route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    true,
                                    failure_meta.map(|failure| failure.code.as_str()),
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                        return SafeLaneTurnOutcome::with_terminal_route(result, route);
                    }
                    SafeLaneRoundDecision::Replan {
                        reason,
                        next_plan_start_tool_index,
                        next_seed_tool_outputs,
                    } => {
                        let seeded_outputs_count = next_seed_tool_outputs.len();
                        state.note_replan(next_plan_start_tool_index, next_seed_tool_outputs);
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "replan_triggered",
                            json!({
                                "round": state.round(),
                                "reason": reason,
                                "restart_tool_index": state.plan_start_tool_index,
                                "seeded_outputs": seeded_outputs_count,
                                "route_decision": route.decision_label(),
                                "route_reason": route.reason.as_str(),
                                "route_source": route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                    }
                }
            }
        }

        state.advance_round();
    }
}

async fn evaluate_safe_lane_round(
    config: &LoongClawConfig,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    state: &SafeLanePlanLoopState,
) -> SafeLaneRoundExecution {
    let plan = build_safe_lane_plan_graph(
        config,
        lane_decision,
        turn,
        state.tool_node_max_attempts(),
        state.plan_start_tool_index,
    );
    let executor = SafeLanePlanNodeExecutor::new(
        turn.tool_intents.as_slice(),
        session_context,
        app_dispatcher,
        binding,
        ingress,
        config.conversation.safe_lane_verify_output_non_empty,
        state.seed_tool_outputs.clone(),
        config
            .conversation
            .tool_result_payload_summary_limit_chars(),
    );
    let report = PlanExecutor::execute(&plan, &executor).await;
    let tool_outputs = executor.tool_outputs_snapshot().await;
    let tool_output_stats = summarize_safe_lane_tool_output_stats(tool_outputs.as_slice());

    SafeLaneRoundExecution {
        report,
        tool_outputs,
        tool_output_stats,
    }
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
    let failed_final_status_events = summary.failed_final_status_events();
    let backpressure_failure_events = summary.backpressure_failure_events();
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
        history_load_status: history.history_load_status,
        history_load_error: history.history_load_error,
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
    binding: ConversationRuntimeBinding<'_>,
) -> SafeLaneGovernorHistorySignals {
    if !config.conversation.safe_lane_session_governor_enabled {
        return SafeLaneGovernorHistorySignals::default();
    }

    let window_turns = config
        .conversation
        .safe_lane_session_governor_window_turns();
    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        return match load_assistant_contents_from_session_window_detailed(
            session_id,
            window_turns,
            binding,
            &memory_config,
        )
        .await
        {
            Ok(assistant_contents) => {
                summarize_governor_history_signals(assistant_contents.iter().map(String::as_str))
            }
            Err(error) => SafeLaneGovernorHistorySignals {
                history_load_status: SafeLaneGovernorHistoryLoadStatus::Unavailable,
                history_load_error: Some(error.code()),
                ..SafeLaneGovernorHistorySignals::default()
            },
        };
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        SafeLaneGovernorHistorySignals::default()
    }
}

fn summarize_governor_history_signals<'a, I>(
    assistant_contents: I,
) -> SafeLaneGovernorHistorySignals
where
    I: IntoIterator<Item = &'a str>,
{
    let projection = summarize_safe_lane_history(assistant_contents);
    SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary: projection.summary,
        final_status_failed_samples: projection.final_status_failed_samples,
        backpressure_failure_samples: projection.backpressure_failure_samples,
    }
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

fn safe_lane_backpressure_budget(config: &LoongClawConfig) -> Option<SafeLaneBackpressureBudget> {
    config
        .conversation
        .safe_lane_backpressure_guard_enabled
        .then(|| {
            SafeLaneBackpressureBudget::new(
                config
                    .conversation
                    .safe_lane_backpressure_max_total_attempts(),
                config.conversation.safe_lane_backpressure_max_replans(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolConfig;
    use crate::context::bootstrap_test_kernel_context;
    use crate::conversation::delegate_support::{
        finalize_async_delegate_spawn_failure, finalize_async_delegate_spawn_failure_with_recovery,
        finalize_delegate_child_terminal_with_recovery,
    };
    use crate::conversation::turn_engine::ToolBatchExecutionIntentTrace;
    use crate::conversation::{
        ConversationTurnObserver, ConversationTurnPhase, ConversationTurnToolState,
    };
    use crate::session::repository::FinalizeSessionTerminalResult;
    use regex::Regex;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_single_tool_intent_direct_binding_reports_no_kernel_context() {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "file.read",
            json!({
                "path": "README.md",
            }),
            Some("root-session"),
            Some("turn-direct-core"),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-direct-core".to_owned(),
            tool_call_id: "call-direct-core".to_owned(),
        };
        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::planned_root_tool_view(),
        );
        let error = execute_single_tool_intent(
            &intent,
            &session_context,
            &crate::conversation::NoopAppToolDispatcher,
            ConversationRuntimeBinding::direct(),
            None,
            2_048,
        )
        .await
        .expect_err("direct core execution should fail closed without kernel context");

        assert_eq!(error.kind, PlanNodeErrorKind::PolicyDenied);
        assert_eq!(error.message, "no_kernel_context");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_single_tool_intent_marks_repairable_file_read_failure_retryable() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "file.read",
            json!({}),
            Some("root-session"),
            Some("turn-file-read-plan-node"),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-file-read-plan-node".to_owned(),
            tool_call_id: "call-file-read-plan-node".to_owned(),
        };
        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::planned_root_tool_view(),
        );

        let error = execute_single_tool_intent(
            &intent,
            &session_context,
            &DefaultAppToolDispatcher::runtime(),
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
            None,
            2_048,
        )
        .await
        .expect_err("repairable file.read preflight should return a plan-node error");

        assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
        assert!(error.message.contains("tool input needs repair"));
        assert!(error.message.contains("file.read payload.path is required"));
    }

    #[cfg(feature = "tool-shell")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_single_tool_intent_marks_repairable_shell_preflight_failure_retryable() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "shell.exec",
            json!({
                "command": "\"ls -la\"",
            }),
            Some("root-session"),
            Some("turn-shell-plan-node"),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-shell-plan-node".to_owned(),
            tool_call_id: "call-shell-plan-node".to_owned(),
        };
        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::planned_root_tool_view(),
        );

        let error = execute_single_tool_intent(
            &intent,
            &session_context,
            &DefaultAppToolDispatcher::runtime(),
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
            None,
            2_048,
        )
        .await
        .expect_err("repairable shell preflight should return a plan-node error");

        assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
        assert!(error.message.contains("tool input needs repair"));
    }

    fn unique_sqlite_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-turn-coordinator-{label}-{}.sqlite3",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn pending_approval_input_parser_accepts_keyword_and_numeric_aliases() {
        assert_eq!(
            parse_pending_approval_input_decision("yes"),
            Some(PendingApprovalInputDecision::RunOnce)
        );
        assert_eq!(
            parse_pending_approval_input_decision("2"),
            Some(PendingApprovalInputDecision::SessionAuto)
        );
        assert_eq!(
            parse_pending_approval_input_decision("本会话全自动"),
            Some(PendingApprovalInputDecision::SessionFull)
        );
        assert_eq!(
            parse_pending_approval_input_decision("esc"),
            Some(PendingApprovalInputDecision::Cancel)
        );
        assert_eq!(
            parse_pending_approval_input_decision("跳过这次"),
            Some(PendingApprovalInputDecision::Cancel)
        );
        assert_eq!(
            parse_pending_approval_input_decision("skip call"),
            Some(PendingApprovalInputDecision::Cancel)
        );
        assert_eq!(parse_pending_approval_input_decision("maybe"), None);
    }

    #[cfg(feature = "memory-sqlite")]
    #[derive(Default)]
    struct ApprovalControlRuntime {
        bootstrap_calls: StdMutex<usize>,
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for ApprovalControlRuntime {
        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(vec![json!({
                "role": "system",
                "content": "approval control test"
            })])
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok("approval handled".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn should not run during approval control replay")
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn_streaming should not run during approval control replay")
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn bootstrap(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _kernel_ctx: &KernelContext,
        ) -> CliResult<crate::conversation::context_engine::ContextEngineBootstrapResult> {
            let mut bootstrap_calls = self
                .bootstrap_calls
                .lock()
                .expect("bootstrap call lock should not be poisoned");
            *bootstrap_calls += 1;
            Ok(Default::default())
        }
    }

    #[cfg(feature = "memory-sqlite")]
    struct CoreReplayRuntime;

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for CoreReplayRuntime {
        fn session_context(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<SessionContext> {
            Err("session_context should not be called for core approval replay".to_owned())
        }

        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Err("build_messages should not run during core approval replay".to_owned())
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Err("request_completion should not run during core approval replay".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Err("request_turn should not run during core approval replay".to_owned())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Err("request_turn_streaming should not run during core approval replay".to_owned())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Err("persist_turn should not run during core approval replay".to_owned())
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_memory_config(label: &str) -> MemoryRuntimeConfig {
        let path = unique_sqlite_path(label);
        let _ = std::fs::remove_file(&path);
        let mut config = LoongClawConfig::default();
        config.memory.sqlite_path = path.display().to_string();
        MemoryRuntimeConfig::from_memory_config(&config.memory)
    }

    #[cfg(feature = "memory-sqlite")]
    fn seed_pending_approval_request(
        repo: &SessionRepository,
        session_id: &str,
        approval_request_id: &str,
        tool_name: &str,
        execution_kind: &str,
    ) {
        repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
            approval_request_id: approval_request_id.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: "turn-pending-approval".to_owned(),
            tool_call_id: "call-pending-approval".to_owned(),
            tool_name: tool_name.to_owned(),
            approval_key: format!("tool:{tool_name}"),
            request_payload_json: json!({
                "session_id": session_id,
                "turn_id": "turn-pending-approval",
                "tool_call_id": "call-pending-approval",
                "tool_name": tool_name,
                "args_json": {},
                "source": "test",
                "execution_kind": execution_kind,
            }),
            governance_snapshot_json: json!({
                "rule_id": "governed_tool_requires_approval",
            }),
        })
        .expect("seed approval request");
    }

    #[cfg(feature = "memory-sqlite")]
    fn unique_workspace_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-turn-coordinator-workspace-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    #[derive(Default)]
    struct RecordingCompactRuntime {
        compact_calls: StdMutex<usize>,
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for RecordingCompactRuntime {
        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn should not be called in compaction tests")
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn_streaming should not be called in compaction tests")
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn compact_context(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _messages: &[Value],
            _kernel_ctx: &KernelContext,
        ) -> CliResult<()> {
            let mut compact_calls = self.compact_calls.lock().expect("compact lock");
            *compact_calls += 1;
            Ok(())
        }
    }

    #[cfg(feature = "memory-sqlite")]
    struct CompactSessionBuildMessagesRuntime {
        session_tool_view: crate::tools::ToolView,
        build_messages_calls: StdMutex<Vec<(bool, crate::tools::ToolView)>>,
        fail_after_first_readback: bool,
    }

    #[cfg(feature = "memory-sqlite")]
    impl CompactSessionBuildMessagesRuntime {
        fn new(session_tool_view: crate::tools::ToolView, fail_after_first_readback: bool) -> Self {
            Self {
                session_tool_view,
                build_messages_calls: StdMutex::new(Vec::new()),
                fail_after_first_readback,
            }
        }
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for CompactSessionBuildMessagesRuntime {
        fn session_context(
            &self,
            _config: &LoongClawConfig,
            session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<SessionContext> {
            Ok(SessionContext::root_with_tool_view(
                session_id,
                self.session_tool_view.clone(),
            ))
        }

        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            include_system_prompt: bool,
            tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            let mut build_messages_calls = self
                .build_messages_calls
                .lock()
                .expect("build_messages lock should not be poisoned");
            build_messages_calls.push((include_system_prompt, tool_view.clone()));
            let call_count = build_messages_calls.len();

            if self.fail_after_first_readback && call_count > 1 {
                return Err("post-compaction readback failed".to_owned());
            }

            let tool_names = tool_view.tool_names().collect::<Vec<_>>();
            let tool_names = tool_names.join(",");

            Ok(vec![json!({
                "role": "system",
                "content": format!(
                    "include_system_prompt={include_system_prompt} tools={tool_names}"
                ),
            })])
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn should not be called in compact_session tests")
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn_streaming should not be called in compact_session tests")
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }

        async fn compact_context(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _messages: &[Value],
            _kernel_ctx: &KernelContext,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct ObserverStreamingRuntime {
        streaming_calls: StdMutex<usize>,
    }

    #[async_trait]
    impl ConversationRuntime for ObserverStreamingRuntime {
        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(vec![json!({
                "role": "system",
                "content": "stay focused"
            })])
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok("completion".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            panic!("request_turn should not be called when observer streaming is enabled")
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            let mut streaming_calls = self
                .streaming_calls
                .lock()
                .expect("streaming call lock should not be poisoned");
            *streaming_calls += 1;

            if let Some(on_token) = on_token {
                on_token(crate::provider::StreamingCallbackData::Text {
                    text: "draft".to_owned(),
                });
            }

            Ok(ProviderTurn {
                assistant_text: "final reply".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            })
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct ObserverFallbackRuntime {
        request_turn_calls: StdMutex<usize>,
        request_turn_streaming_calls: StdMutex<usize>,
    }

    #[async_trait]
    impl ConversationRuntime for ObserverFallbackRuntime {
        async fn build_messages(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(vec![json!({
                "role": "system",
                "content": "stay focused"
            })])
        }

        async fn request_completion(
            &self,
            _config: &LoongClawConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok("completion".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            let mut request_turn_calls = self
                .request_turn_calls
                .lock()
                .expect("request-turn call lock should not be poisoned");
            *request_turn_calls += 1;

            Ok(ProviderTurn {
                assistant_text: "final reply".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            })
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongClawConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &crate::tools::ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            let mut request_turn_streaming_calls = self
                .request_turn_streaming_calls
                .lock()
                .expect("request-turn-streaming call lock should not be poisoned");
            *request_turn_streaming_calls += 1;
            panic!("request_turn_streaming should not be called for unsupported transports")
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingTurnObserver {
        phase_events: StdMutex<Vec<ConversationTurnPhaseEvent>>,
        tool_events: StdMutex<Vec<ConversationTurnToolEvent>>,
        token_events: StdMutex<Vec<crate::acp::StreamingTokenEvent>>,
    }

    impl ConversationTurnObserver for RecordingTurnObserver {
        fn on_phase(&self, event: ConversationTurnPhaseEvent) {
            let mut phase_events = self
                .phase_events
                .lock()
                .expect("phase event lock should not be poisoned");
            phase_events.push(event);
        }

        fn on_tool(&self, event: ConversationTurnToolEvent) {
            let mut tool_events = self
                .tool_events
                .lock()
                .expect("tool event lock should not be poisoned");
            tool_events.push(event);
        }

        fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
            let mut token_events = self
                .token_events
                .lock()
                .expect("token event lock should not be poisoned");
            token_events.push(event);
        }
    }

    #[tokio::test]
    async fn handle_turn_with_observer_uses_streaming_request_and_emits_live_events() {
        let mut config = LoongClawConfig::default();
        config.provider.kind = crate::config::ProviderKind::Anthropic;

        let runtime = ObserverStreamingRuntime::default();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");
        let reply = ConversationTurnCoordinator::new()
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                Some(observer_handle),
            )
            .await
            .expect("observer turn should succeed");

        assert_eq!(reply, "final reply");

        let streaming_calls = runtime
            .streaming_calls
            .lock()
            .expect("streaming call lock should not be poisoned");
        assert_eq!(*streaming_calls, 1);

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();
        assert_eq!(
            phase_names,
            vec![
                ConversationTurnPhase::Preparing,
                ConversationTurnPhase::ContextReady,
                ConversationTurnPhase::RequestingProvider,
                ConversationTurnPhase::FinalizingReply,
                ConversationTurnPhase::Completed,
            ]
        );

        let token_events = observer
            .token_events
            .lock()
            .expect("token event lock should not be poisoned");
        assert_eq!(token_events.len(), 1);
        assert_eq!(token_events[0].event_type, "text_delta");
        assert_eq!(token_events[0].delta.text.as_deref(), Some("draft"));
    }

    #[tokio::test]
    async fn handle_turn_with_observer_falls_back_when_streaming_events_are_unsupported() {
        let mut config = LoongClawConfig::default();
        config.provider.kind = crate::config::ProviderKind::Openai;
        config.provider.wire_api = crate::config::ProviderWireApi::Responses;

        let runtime = ObserverFallbackRuntime::default();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");
        let reply = ConversationTurnCoordinator::new()
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                Some(observer_handle),
            )
            .await
            .expect("observer turn should succeed");

        assert_eq!(reply, "final reply");

        let request_turn_calls = runtime
            .request_turn_calls
            .lock()
            .expect("request-turn call lock should not be poisoned");
        assert_eq!(*request_turn_calls, 1);

        let request_turn_streaming_calls = runtime
            .request_turn_streaming_calls
            .lock()
            .expect("request-turn-streaming call lock should not be poisoned");
        assert_eq!(*request_turn_streaming_calls, 0);

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();
        assert_eq!(
            phase_names,
            vec![
                ConversationTurnPhase::Preparing,
                ConversationTurnPhase::ContextReady,
                ConversationTurnPhase::RequestingProvider,
                ConversationTurnPhase::FinalizingReply,
                ConversationTurnPhase::Completed,
            ]
        );

        let token_events = observer
            .token_events
            .lock()
            .expect("token event lock should not be poisoned");
        assert!(
            token_events.is_empty(),
            "unsupported transports should not emit streaming token events: {token_events:#?}"
        );
    }

    #[tokio::test]
    async fn handle_turn_with_observer_emits_lifecycle_for_explicit_acp_inline_message() {
        let config = LoongClawConfig::default();
        let runtime = ObserverStreamingRuntime::default();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::explicit();
        let address = ConversationSessionAddress::from_session_id("observer-session");
        let reply = ConversationTurnCoordinator::new()
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::InlineMessage,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                Some(observer_handle),
            )
            .await
            .expect("ACP inline reply should succeed");

        let expected_reply =
            format_provider_error_reply("ACP is disabled by policy (`acp.enabled=false`)");
        assert_eq!(reply, expected_reply);

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();
        assert_eq!(
            phase_names,
            vec![
                ConversationTurnPhase::Preparing,
                ConversationTurnPhase::FinalizingReply,
                ConversationTurnPhase::Completed,
            ]
        );

        let tool_events = observer
            .tool_events
            .lock()
            .expect("tool event lock should not be poisoned");
        assert!(tool_events.is_empty());

        let token_events = observer
            .token_events
            .lock()
            .expect("token event lock should not be poisoned");
        assert!(token_events.is_empty());

        let streaming_calls = runtime
            .streaming_calls
            .lock()
            .expect("streaming call lock should not be poisoned");
        assert_eq!(*streaming_calls, 0);
    }

    #[tokio::test]
    async fn handle_turn_with_ingress_and_observer_marks_failed_when_runtime_bootstrap_fails() {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-observer-runtime-ingress".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");

        let result = coordinator
            .handle_turn_with_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                Some(observer_handle),
            )
            .await;
        let _error = result.expect_err("missing runtime bootstrap should fail");

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();
        assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
    }

    #[tokio::test]
    async fn handle_turn_with_observer_marks_failed_when_runtime_bootstrap_fails() {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-observer-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");

        let result = coordinator
            .handle_turn_with_address_and_acp_options_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                Some(observer_handle),
            )
            .await;
        let _error = result.expect_err("missing runtime bootstrap should fail");

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();
        assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
    }

    #[tokio::test]
    async fn handle_production_turn_with_observer_rejects_direct_binding_before_runtime_bootstrap()
    {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-observer-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");

        let result = coordinator
            .handle_production_turn_with_address_and_acp_options_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                Some(observer_handle),
            )
            .await;
        let error = result.expect_err("direct production binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
    }

    #[tokio::test]
    async fn handle_production_turn_with_runtime_rejects_direct_binding_before_provider_request() {
        let mut config = LoongClawConfig::default();
        config.provider.kind = crate::config::ProviderKind::Anthropic;

        let runtime = ObserverStreamingRuntime::default();
        let coordinator = ConversationTurnCoordinator::new();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("observer-session");

        let result = coordinator
            .handle_production_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "say hello",
                ProviderErrorMode::Propagate,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                Some(observer_handle),
            )
            .await;
        let error = result.expect_err("direct production binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );

        let streaming_calls = runtime
            .streaming_calls
            .lock()
            .expect("streaming call lock should not be poisoned");

        assert_eq!(*streaming_calls, 0);

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(phase_names, vec![ConversationTurnPhase::Failed]);
    }

    #[tokio::test]
    async fn compact_production_session_rejects_direct_binding_before_runtime_bootstrap() {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let result = coordinator
            .compact_production_session(
                &config,
                "maintenance-session",
                ConversationRuntimeBinding::direct(),
            )
            .await;
        let error = result.expect_err("direct production maintenance binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );
    }

    #[tokio::test]
    async fn repair_production_turn_checkpoint_tail_rejects_direct_binding_before_runtime_bootstrap()
     {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let result = coordinator
            .repair_production_turn_checkpoint_tail(
                &config,
                "maintenance-session",
                ConversationRuntimeBinding::direct(),
            )
            .await;
        let error = result.expect_err("direct production maintenance binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );
    }

    #[tokio::test]
    async fn load_production_turn_checkpoint_diagnostics_rejects_direct_binding_before_runtime_bootstrap()
     {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let result = coordinator
            .load_production_turn_checkpoint_diagnostics(
                &config,
                "maintenance-session",
                ConversationRuntimeBinding::direct(),
            )
            .await;
        let error = result.expect_err("direct production maintenance binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );
    }

    #[tokio::test]
    async fn probe_production_turn_checkpoint_tail_runtime_gate_rejects_direct_binding_before_runtime_bootstrap()
     {
        let mut config = LoongClawConfig::default();
        config.conversation.context_engine = Some("missing-maintenance-runtime".to_owned());

        let coordinator = ConversationTurnCoordinator::new();
        let result = coordinator
            .probe_production_turn_checkpoint_tail_runtime_gate(
                &config,
                "maintenance-session",
                ConversationRuntimeBinding::direct(),
            )
            .await;
        let error = result.expect_err("direct production maintenance binding should fail");

        assert_eq!(
            error,
            PRODUCTION_CONVERSATION_RUNTIME_REQUIRES_KERNEL_BINDING
        );
    }

    #[test]
    fn generic_direct_capable_coordinator_entrypoints_are_not_public_api() {
        let source = include_str!("turn_coordinator.rs");
        let function_names = [
            "compact_session",
            "compact_session_with_runtime",
            "handle_turn",
            "handle_turn_with_ingress",
            "handle_turn_with_acp_options",
            "repair_turn_checkpoint_tail",
            "probe_turn_checkpoint_tail_runtime_gate",
            "probe_turn_checkpoint_tail_runtime_gate_with_limit",
            "handle_turn_with_acp_event_sink",
            "handle_turn_with_address",
            "handle_turn_with_address_and_acp_event_sink",
            "handle_turn_with_address_and_acp_options_and_ingress",
            "handle_turn_with_address_and_acp_options",
            "handle_turn_with_address_and_acp_options_and_ingress_and_observer",
            "handle_turn_with_address_and_acp_options_and_observer",
            "handle_turn_with_runtime",
            "handle_turn_with_runtime_and_ingress",
            "repair_turn_checkpoint_tail_with_runtime",
            "probe_turn_checkpoint_tail_runtime_gate_with_runtime",
            "probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit",
            "handle_turn_with_runtime_and_acp_options",
            "handle_turn_with_runtime_and_acp_event_sink",
            "handle_turn_with_runtime_and_address",
            "handle_turn_with_runtime_and_address_and_acp_options",
            "handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer",
            "handle_turn_with_runtime_and_address_and_acp_event_sink",
        ];
        let signature_pattern = Regex::new(
            r"(?m)^(?:(?:\s*///[^\n]*\n)|(?:\s*#\[[^\n]+\]\s*\n))*\s*(?P<visibility>pub(?:\([^)]*\))?\s+)?async\s+fn\s+(?P<name>[A-Za-z0-9_]+)\b",
        )
        .expect("signature regex should compile");
        let mut public_function_names = Vec::new();

        for captures in signature_pattern.captures_iter(source) {
            let Some(name_match) = captures.name("name") else {
                continue;
            };
            let visibility = captures
                .name("visibility")
                .map(|match_value| match_value.as_str().trim())
                .unwrap_or("");
            if visibility != "pub" {
                continue;
            }

            let function_name = name_match.as_str().to_owned();
            public_function_names.push(function_name);
        }

        for function_name in function_names {
            let is_public = public_function_names
                .iter()
                .any(|public_name| public_name == function_name);
            assert!(
                !is_public,
                "generic direct-capable coordinator seam should stay internal: {function_name}"
            );
        }
    }

    #[test]
    fn build_provider_turn_tool_terminal_events_prefers_trace_outcomes_over_generic_fallbacks() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "sessions_list".to_owned(),
                    args_json: json!({}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "session_status".to_owned(),
                    args_json: json!({"session_id": "session-a"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        };
        let turn_result = TurnResult::ToolError(TurnFailure::retryable(
            "tool_execution_failed",
            "second tool failed",
        ));
        let trace = ToolBatchExecutionTrace {
            total_intents: 2,
            parallel_execution_enabled: false,
            parallel_execution_max_in_flight: 1,
            observed_peak_in_flight: 1,
            observed_wall_time_ms: 10,
            segments: Vec::new(),
            decision_records: Vec::new(),
            outcome_records: Vec::new(),
            intent_outcomes: vec![
                ToolBatchExecutionIntentTrace {
                    tool_call_id: "call-1".to_owned(),
                    tool_name: "sessions_list".to_owned(),
                    status: ToolBatchExecutionIntentStatus::Completed,
                    detail: None,
                },
                ToolBatchExecutionIntentTrace {
                    tool_call_id: "call-2".to_owned(),
                    tool_name: "session_status".to_owned(),
                    status: ToolBatchExecutionIntentStatus::Failed,
                    detail: Some("second tool failed".to_owned()),
                },
            ],
        };

        let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, Some(&trace));

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].tool_call_id, "call-1");
        assert_eq!(events[0].state, ConversationTurnToolState::Completed);
        assert_eq!(events[1].tool_call_id, "call-2");
        assert_eq!(events[1].state, ConversationTurnToolState::Failed);
        assert_eq!(events[1].detail.as_deref(), Some("second tool failed"));
    }

    #[test]
    fn build_provider_turn_tool_terminal_events_attach_canonical_shell_request_summary() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "ls /root"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-shell".to_owned(),
            }],
            raw_meta: Value::Null,
        };
        let turn_result = TurnResult::ToolDenied(TurnFailure::policy_denied(
            "shell_policy_denied",
            "policy_denied: command contains embedded whitespace",
        ));

        let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, None);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_call_id, "call-shell");
        assert_eq!(events[0].state, ConversationTurnToolState::Denied);
        let request_summary =
            summarize_tool_event_request(&turn.tool_intents[0]).expect("request summary");
        let request_summary_json: Value =
            serde_json::from_str(&request_summary).expect("request summary should be valid json");
        assert_eq!(
            request_summary_json,
            json!({
                "tool": "shell.exec",
                "request": {"command": "ls", "args_redacted": 1}
            })
        );
    }

    #[test]
    fn summarize_failed_provider_lane_tool_request_preserves_multi_intent_context_without_trace() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "Cargo.toml"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "shell.exec".to_owned(),
                    args_json: json!({"command": "ls /root"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        };

        let request_summary = summarize_provider_lane_tool_request(
            &turn,
            &TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure")),
            None,
        )
        .expect("multi-intent failures should retain a request summary");
        let request_summary_json: Value =
            serde_json::from_str(&request_summary).expect("request summary should be valid json");
        let request_entries = request_summary_json
            .as_array()
            .expect("multi-intent request summary should be an array");

        assert_eq!(request_entries.len(), 2);
        assert_eq!(request_entries[0]["tool"], "file.read");
        assert_eq!(request_entries[1]["tool"], "shell.exec");
        assert_eq!(request_entries[1]["request"]["command"], "ls");
        assert_eq!(request_entries[1]["request"]["args_redacted"], 1);
    }
    #[cfg(feature = "memory-sqlite")]
    fn finalize_recovered_child(
        repo: &SessionRepository,
        expected_state: SessionState,
    ) -> FinalizeSessionTerminalResult {
        let frozen_result = crate::session::frozen_result::FrozenResult {
            content: crate::session::frozen_result::FrozenContent::Text(
                "delegate_recovered".to_owned(),
            ),
            captured_at: std::time::SystemTime::now(),
            byte_len: "delegate_recovered".len(),
            truncated: false,
        };
        repo.finalize_session_terminal_if_current(
            "child-session",
            expected_state,
            FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some("delegate_recovered".to_owned()),
                event_kind: RECOVERY_EVENT_KIND.to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "recovery_kind": "forced_recovery",
                    "recovered_state": "failed",
                }),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": "delegate_recovered"
                }),
                frozen_result: Some(frozen_result),
            },
        )
        .expect("recover child terminal state")
        .expect("recovery should transition child")
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn finalize_delegate_child_terminal_with_recovery_does_not_overwrite_recovered_failure() {
        let memory_config = sqlite_memory_config("recovered-running-child");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child session");

        let recovered = finalize_recovered_child(&repo, SessionState::Running);
        assert_eq!(recovered.session.state, SessionState::Failed);
        assert_eq!(recovered.terminal_outcome.status, "error");

        finalize_delegate_child_terminal_with_recovery(
            &repo,
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "turn_count": 1,
                    "duration_ms": 12,
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "final_output": "late success",
                }),
                frozen_result: None,
            },
        )
        .expect("stale running finalizer should no-op");

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Failed);
        assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        let event_kinds: Vec<&str> = events
            .iter()
            .map(|event| event.event_kind.as_str())
            .collect();
        assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
        assert!(!event_kinds.contains(&"delegate_completed"));

        let terminal_outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(terminal_outcome.status, "error");
        assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
        assert_eq!(
            terminal_outcome
                .frozen_result
                .expect("frozen result")
                .content,
            crate::session::frozen_result::FrozenContent::Text("delegate_recovered".to_owned())
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn finalize_async_delegate_spawn_failure_does_not_overwrite_recovered_failure() {
        let memory_config = sqlite_memory_config("recovered-ready-child");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
            owner_kind: None,
            depth: 1,
            max_depth: 1,
            active_children: 0,
            max_active_children: 1,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec![
                "file.read".to_owned(),
                "file.write".to_owned(),
                "file.edit".to_owned(),
            ],
            workspace_root: None,
            runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
        };
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child session");

        let recovered = finalize_recovered_child(&repo, SessionState::Ready);
        assert_eq!(recovered.session.state, SessionState::Failed);
        assert_eq!(recovered.terminal_outcome.status, "error");

        finalize_async_delegate_spawn_failure(
            &memory_config,
            "child-session",
            "root-session",
            Some("Child".to_owned()),
            None,
            &execution,
            crate::config::ToolConfig::default()
                .delegate
                .max_frozen_bytes,
            "spawn unavailable".to_owned(),
        )
        .expect("stale queued spawn failure finalizer should no-op");

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Failed);
        assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

        let events = repo
            .list_recent_events("child-session", 10)
            .expect("list child events");
        let event_kinds: Vec<&str> = events
            .iter()
            .map(|event| event.event_kind.as_str())
            .collect();
        assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
        assert!(!event_kinds.contains(&"delegate_spawn_failed"));

        let terminal_outcome = repo
            .load_terminal_outcome("child-session")
            .expect("load terminal outcome")
            .expect("terminal outcome row");
        assert_eq!(terminal_outcome.status, "error");
        assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
        assert_eq!(
            terminal_outcome
                .frozen_result
                .expect("frozen result")
                .content,
            crate::session::frozen_result::FrozenContent::Text("delegate_recovered".to_owned())
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn finalize_delegate_child_terminal_with_recovery_errors_when_child_session_missing() {
        let memory_config = sqlite_memory_config("missing-running-child");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let error = finalize_delegate_child_terminal_with_recovery(
            &repo,
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "turn_count": 1,
                    "duration_ms": 12,
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "final_output": "late success",
                }),
                frozen_result: None,
            },
        )
        .expect_err("missing child session should not be treated as stale");

        assert!(error.contains("session `child-session` not found"));
        assert!(error.contains("delegate_terminal_recovery_skipped_from_state: missing"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn finalize_async_delegate_spawn_failure_with_recovery_errors_when_child_session_missing() {
        let memory_config = sqlite_memory_config("missing-ready-child");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
            owner_kind: None,
            depth: 1,
            max_depth: 1,
            active_children: 0,
            max_active_children: 1,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec![
                "file.read".to_owned(),
                "file.write".to_owned(),
                "file.edit".to_owned(),
            ],
            workspace_root: None,
            runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
        };
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let error = finalize_async_delegate_spawn_failure_with_recovery(
            &memory_config,
            "child-session",
            "root-session",
            Some("Child".to_owned()),
            None,
            &execution,
            crate::config::ToolConfig::default()
                .delegate
                .max_frozen_bytes,
            "spawn unavailable".to_owned(),
        )
        .expect_err("missing child session should not bypass spawn failure recovery");

        assert!(error.contains("session `child-session` not found"));
        assert!(error.contains("delegate_async_spawn_recovery_skipped_from_state: missing"));
        assert_eq!(
            repo.load_session("child-session")
                .expect("load child session"),
            None
        );
    }

    #[test]
    fn build_turn_reply_followup_messages_include_truncation_hint_for_truncated_tool_results() {
        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
            },
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
    fn build_turn_reply_followup_messages_do_not_include_truncation_hint_for_failure() {
        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolFailure {
                reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
            },
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
    fn build_turn_reply_followup_messages_promotes_external_skill_invoke_to_system_context() {
        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
            },
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
    fn build_turn_reply_followup_messages_rejects_truncated_external_skill_invoke_payload() {
        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":true}"#.to_owned(),
            },
            "summarize note.md",
        );

        assert!(
            !messages.iter().any(|message| message.get("role")
                == Some(&Value::String("system".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content
                        .contains("Follow the managed skill instruction before answering."))
                    .unwrap_or(false)),
            "truncated invoke payload must not activate managed skill system context: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .filter(
                    |message| message.get("role") == Some(&Value::String("assistant".to_owned()))
                )
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .any(|content| content.contains("[tool_result]\n[ok]")),
            "truncated invoke payload should stay as ordinary assistant tool_result content: {messages:?}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn persist_runtime_self_continuity_for_compaction_merges_live_and_stored_delegate_continuity() {
        let workspace_root = unique_workspace_root("merged-runtime-self-continuity");
        let memory_config = sqlite_memory_config("merged-runtime-self-continuity");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        let root_session_id = "root-session";
        let child_session_id = "delegate:child-session";
        let live_agents_text = "Keep standing instructions visible.";
        let stored_identity_text = "# Identity\n\n- Name: Stored continuity identity";
        let mut config = LoongClawConfig::default();

        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(workspace_root.join("AGENTS.md"), live_agents_text).expect("write AGENTS");
        config.memory.sqlite_path = sqlite_path;
        config.tools.file_root = Some(workspace_root.display().to_string());

        repo.create_session(NewSessionRecord {
            session_id: root_session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: child_session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(root_session_id.to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child session");

        let stored_continuity = runtime_self_continuity::RuntimeSelfContinuity {
            runtime_self: crate::runtime_self::RuntimeSelfModel {
                identity_context: vec![stored_identity_text.to_owned()],
                ..Default::default()
            },
            resolved_identity: Some(crate::runtime_identity::ResolvedRuntimeIdentity {
                source: crate::runtime_identity::RuntimeIdentitySource::LegacyProfileNoteImport,
                content: stored_identity_text.to_owned(),
            }),
            session_profile_projection: None,
        };
        repo.append_event(NewSessionEvent {
            session_id: child_session_id.to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some(root_session_id.to_owned()),
            payload_json: json!({
                "runtime_self_continuity": stored_continuity,
            }),
        })
        .expect("append delegate event");

        persist_runtime_self_continuity_for_compaction(&config, child_session_id)
            .expect("persist merged runtime self continuity");

        let recent_events = repo
            .list_recent_events(child_session_id, 10)
            .expect("list recent events");
        let persisted_event = recent_events
            .iter()
            .rev()
            .find(|event| {
                event.event_kind == runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
            })
            .expect("persisted continuity event");
        let persisted_continuity =
            runtime_self_continuity::runtime_self_continuity_from_event_payload(
                &persisted_event.payload_json,
            )
            .expect("decode persisted continuity payload");

        assert_eq!(
            persisted_continuity.runtime_self.standing_instructions,
            vec![live_agents_text.to_owned()]
        );
        assert_eq!(
            persisted_continuity.runtime_self.identity_context,
            vec![stored_identity_text.to_owned()]
        );
        assert_eq!(
            persisted_continuity
                .resolved_identity
                .as_ref()
                .map(|value| value.content.as_str()),
            Some(stored_identity_text)
        );

        let _ = std::fs::remove_dir_all(&workspace_root);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn persist_runtime_self_continuity_for_compaction_reconstructs_legacy_delegate_session_row() {
        let workspace_root = unique_workspace_root("legacy-delegate-session-row");
        let memory_config = sqlite_memory_config("legacy-delegate-session-row");
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        let root_session_id = "root-session";
        let child_session_id = "delegate:legacy-child";
        let mut config = LoongClawConfig::default();

        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .clone();
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(
            workspace_root.join("AGENTS.md"),
            "Keep continuity explicit.",
        )
        .expect("write AGENTS");
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.tools.file_root = Some(workspace_root.display().to_string());

        repo.create_session(NewSessionRecord {
            session_id: root_session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite connection");
        conn.execute(
            "INSERT INTO turns(session_id, session_turn_index, role, content, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![child_session_id, 1_i64, "assistant", "legacy turn", 1_i64],
        )
        .expect("insert legacy turn");
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                child_session_id,
                "delegate_started",
                root_session_id,
                json!({}).to_string(),
                2_i64
            ],
        )
        .expect("insert legacy delegate event");
        drop(conn);

        persist_runtime_self_continuity_for_compaction(&config, child_session_id)
            .expect("persist runtime self continuity");

        let reconstructed_session = repo
            .load_session(child_session_id)
            .expect("load reconstructed session")
            .expect("reconstructed session row");

        assert_eq!(reconstructed_session.kind, SessionKind::DelegateChild);
        assert_eq!(
            reconstructed_session.parent_session_id.as_deref(),
            Some(root_session_id)
        );

        let _ = std::fs::remove_dir_all(&workspace_root);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn maybe_compact_context_fails_open_when_runtime_self_continuity_persist_cannot_reconstruct_delegate_lineage()
     {
        let workspace_root = unique_workspace_root("compaction-fail-open");
        let sqlite_path = unique_sqlite_path("compaction-fail-open");
        let runtime = RecordingCompactRuntime::default();
        let mut config = LoongClawConfig::default();

        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        std::fs::write(
            workspace_root.join("AGENTS.md"),
            "Keep continuity explicit.",
        )
        .expect("write AGENTS");
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.tools.file_root = Some(workspace_root.display().to_string());
        config.conversation.compact_min_messages = Some(1);
        config.conversation.compact_trigger_estimated_tokens = Some(1);
        config.conversation.compact_fail_open = true;

        let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
            .expect("bootstrap kernel context");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let runtime_handle = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "trigger compaction"}),
        ];
        let outcome = runtime_handle.block_on(maybe_compact_context(
            &config,
            &runtime,
            "delegate:missing-lineage",
            &messages,
            Some(16),
            binding,
            false,
        ));

        assert_eq!(
            outcome.expect("compaction should fail open"),
            ContextCompactionOutcome::FailedOpen
        );
        let compact_calls = runtime.compact_calls.lock().expect("compact lock");
        assert_eq!(*compact_calls, 0);

        let _ = std::fs::remove_dir_all(&workspace_root);
        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn maybe_compact_context_fails_open_when_durable_flush_cannot_write_workspace_export() {
        let workspace_root_parent = unique_workspace_root("compaction-durable-flush-fail-open");
        let workspace_root_file = workspace_root_parent.join("workspace-root-file");
        let sqlite_path = unique_sqlite_path("compaction-durable-flush-fail-open");
        let runtime = RecordingCompactRuntime::default();
        let mut config = LoongClawConfig::default();

        std::fs::create_dir_all(&workspace_root_parent).expect("create workspace root parent");
        std::fs::write(
            workspace_root_parent.join("AGENTS.md"),
            "Keep continuity explicit.",
        )
        .expect("write AGENTS");
        std::fs::write(&workspace_root_file, "not a workspace directory")
            .expect("write workspace root file");
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.tools.file_root = Some(workspace_root_file.display().to_string());
        config.memory.sliding_window = 1;
        config.conversation.compact_min_messages = Some(1);
        config.conversation.compact_trigger_estimated_tokens = Some(1);
        config.conversation.compact_fail_open = true;

        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::memory::append_turn_direct(
            "session-durable-flush-fail-open",
            "user",
            "remember the deployment cutoff",
            &memory_config,
        )
        .expect("append user turn");
        crate::memory::append_turn_direct(
            "session-durable-flush-fail-open",
            "assistant",
            "deployment cutoff is tonight",
            &memory_config,
        )
        .expect("append assistant turn");

        let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
            .expect("bootstrap kernel context");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let runtime_handle = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "trigger compaction"}),
        ];
        let outcome = runtime_handle.block_on(maybe_compact_context(
            &config,
            &runtime,
            "session-durable-flush-fail-open",
            &messages,
            Some(16),
            binding,
            false,
        ));

        assert_eq!(
            outcome.expect("compaction should fail open"),
            ContextCompactionOutcome::FailedOpen
        );
        let compact_calls = runtime.compact_calls.lock().expect("compact lock");
        assert_eq!(*compact_calls, 0);

        let _ = std::fs::remove_dir_all(&workspace_root_parent);
        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_session_uses_session_context_tool_view_and_turn_like_build_flags() {
        let mut config = LoongClawConfig::default();
        let sqlite_path = unique_sqlite_path("compact-session-build-messages");
        let _ = std::fs::remove_file(&sqlite_path);
        config.memory.sqlite_path = sqlite_path.display().to_string();

        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::memory::append_turn_direct(
            "compact-session-build-messages",
            "user",
            "remember this detail",
            &memory_config,
        )
        .expect("append user turn");

        let expected_tool_view = crate::tools::ToolView::from_tool_names(["status.inspect"]);
        let runtime = CompactSessionBuildMessagesRuntime::new(expected_tool_view.clone(), false);
        let kernel_ctx = bootstrap_test_kernel_context("compact-session-build-messages", 3600)
            .expect("bootstrap kernel context");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let coordinator = ConversationTurnCoordinator::new();

        let report = coordinator
            .compact_session_with_runtime(
                &config,
                "compact-session-build-messages",
                &runtime,
                binding,
            )
            .await
            .expect("manual compaction should succeed");

        assert!(report.was_skipped());

        let build_messages_calls = runtime
            .build_messages_calls
            .lock()
            .expect("build_messages lock should not be poisoned");
        assert_eq!(build_messages_calls.len(), 2);
        assert!(
            build_messages_calls
                .iter()
                .all(|(include_system_prompt, _tool_view)| *include_system_prompt),
            "compact_session should mirror turn assembly by keeping the system prompt enabled"
        );
        assert!(
            build_messages_calls
                .iter()
                .all(|(_include_system_prompt, tool_view)| { *tool_view == expected_tool_view }),
            "compact_session should reuse the session-context tool view for both snapshots"
        );

        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_session_skips_when_post_compaction_readback_fails() {
        let mut config = LoongClawConfig::default();
        let sqlite_path = unique_sqlite_path("compact-session-readback-fail");
        let _ = std::fs::remove_file(&sqlite_path);
        config.memory.sqlite_path = sqlite_path.display().to_string();

        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::memory::append_turn_direct(
            "compact-session-readback-fail",
            "user",
            "keep the context intact",
            &memory_config,
        )
        .expect("append user turn");

        let runtime = CompactSessionBuildMessagesRuntime::new(
            crate::tools::ToolView::from_tool_names(["status.inspect"]),
            true,
        );
        let kernel_ctx = bootstrap_test_kernel_context("compact-session-readback-fail", 3600)
            .expect("bootstrap kernel context");
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
        let coordinator = ConversationTurnCoordinator::new();

        let report = coordinator
            .compact_session_with_runtime(
                &config,
                "compact-session-readback-fail",
                &runtime,
                binding,
            )
            .await
            .expect("manual compaction should degrade to skipped");

        assert!(report.was_skipped());
        assert_eq!(
            report.estimated_tokens_after,
            report.estimated_tokens_before
        );

        let build_messages_calls = runtime
            .build_messages_calls
            .lock()
            .expect("build_messages lock should not be poisoned");
        assert_eq!(build_messages_calls.len(), 2);

        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[test]
    fn build_turn_reply_followup_messages_reduces_file_read_payload_summary() {
        let content = (0..96)
            .map(|index| format!("line {index}: {}", "x".repeat(48)))
            .collect::<Vec<_>>()
            .join("\n");
        let payload_summary = serde_json::json!({
            "adapter": "core-tools",
            "tool_name": "file.read",
            "path": "/repo/README.md",
            "bytes": 8_192,
            "truncated": false,
            "content": content,
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-file",
                "payload_summary": payload_summary,
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        );

        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "summarize README.md",
        );

        let assistant_tool_result = messages
            .iter()
            .find(|message| {
                message.get("role") == Some(&Value::String("assistant".to_owned()))
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
            })
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("assistant tool_result followup message should exist");
        let line = assistant_tool_result
            .lines()
            .nth(1)
            .expect("assistant tool_result should keep payload line");
        let envelope: Value = serde_json::from_str(
            line.strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("reduced followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("file.read payload summary should stay valid json");

        assert_eq!(envelope["tool"], "file.read");
        assert_eq!(envelope["payload_truncated"], true);
        assert_eq!(summary["path"], "/repo/README.md");
        assert_eq!(summary["bytes"], 8_192);
        assert_eq!(summary["truncated"], false);
        assert!(summary.get("content_preview").is_some());
        assert!(summary.get("content_chars").is_some());
        assert_eq!(summary["content_truncated"], true);
    }

    #[test]
    fn build_turn_reply_followup_messages_reduces_shell_exec_payload_summary() {
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "shell.exec",
                "tool_call_id": "call-shell",
                "payload_summary": serde_json::json!({
                    "adapter": "core-tools",
                    "tool_name": "shell.exec",
                    "command": "cargo",
                    "args": ["test", "--workspace"],
                    "cwd": "/repo",
                    "exit_code": 0,
                    "stdout": (0..80)
                        .map(|index| format!("stdout line {index}: {}", "x".repeat(40)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    "stderr": (0..48)
                        .map(|index| format!("stderr line {index}: {}", "e".repeat(32)))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .to_string(),
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        );

        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "summarize the test run",
        );

        let (envelope, summary) =
            crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

        assert_eq!(envelope["tool"], "shell.exec");
        assert_eq!(envelope["payload_truncated"], true);
        assert_eq!(summary["command"], "cargo");
        assert_eq!(summary["exit_code"], 0);
        assert!(summary.get("stdout_preview").is_some());
        assert!(summary.get("stdout_chars").is_some());
        assert_eq!(summary["stdout_truncated"], true);
        assert!(summary.get("stderr_preview").is_some());
        assert!(summary.get("stderr_chars").is_some());
        assert_eq!(summary["stderr_truncated"], true);
        assert!(
            summary["stdout_preview"]
                .as_str()
                .expect("stdout preview should exist")
                .contains("stdout line 0"),
            "expected compact stdout preview, got: {summary:?}"
        );
        assert!(
            summary["stderr_preview"]
                .as_str()
                .expect("stderr preview should exist")
                .contains("stderr line 0"),
            "expected compact stderr preview, got: {summary:?}"
        );
    }

    #[test]
    fn build_turn_reply_followup_messages_compacts_tool_search_payload_summary() {
        let payload_summary = serde_json::json!({
            "adapter": "core-tools",
            "tool_name": "tool.search",
            "query": "read repo file",
            "returned": 2,
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "tags": ["core", "file", "read"],
                    "why": ["summary matches query", "tag matches read"],
                    "lease": "lease-file"
                },
                {
                    "tool_id": "shell.exec",
                    "summary": "Execute a shell command in the workspace.",
                    "argument_hint": "command:string,args?:string[]",
                    "required_fields": ["command"],
                    "required_field_groups": [["command"]],
                    "tags": ["core", "shell", "exec"],
                    "why": ["summary matches query", "tag matches exec"],
                    "lease": "lease-shell"
                }
            ]
        });
        let payload_summary_str = payload_summary.to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "tool.search",
                "tool_call_id": "call-search",
                "payload_chars": 2_048,
                "payload_summary": payload_summary_str,
                "payload_truncated": false
            })
        );

        let messages = build_turn_reply_followup_messages(
            &[serde_json::json!({
                "role": "system",
                "content": "sys"
            })],
            "preface",
            ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "find the right tool",
        );

        let (envelope, summary) =
            crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);
        let summary_str = envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json");
        let results = summary["results"]
            .as_array()
            .expect("results should be an array");
        let first = &results[0];

        assert_eq!(envelope["tool"], "tool.search");
        assert_eq!(envelope["payload_truncated"], false);
        assert_ne!(summary_str, payload_summary.to_string());
        assert_eq!(summary["query"], "read repo file");
        assert!(summary.get("adapter").is_none());
        assert!(summary.get("tool_name").is_none());
        assert_eq!(summary["returned"], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(first["tool_id"], "file.read");
        assert_eq!(first["lease"], "lease-file");
        for entry in results {
            assert!(entry.get("tool_id").and_then(Value::as_str).is_some());
            assert!(entry.get("summary").and_then(Value::as_str).is_some());
            assert!(entry.get("argument_hint").and_then(Value::as_str).is_some());
            assert!(
                entry
                    .get("required_fields")
                    .and_then(Value::as_array)
                    .is_some()
            );
            assert!(
                entry
                    .get("required_field_groups")
                    .and_then(Value::as_array)
                    .is_some()
            );
            assert!(entry.get("lease").and_then(Value::as_str).is_some());
            assert!(entry.get("tags").is_none());
            assert!(entry.get("why").is_none());
        }
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn pending_approval_control_turn_bootstraps_once_and_emits_terminal_phases_once() {
        let runtime = ApprovalControlRuntime::default();
        let observer = Arc::new(RecordingTurnObserver::default());
        let observer_handle: ConversationTurnObserverHandle = observer.clone();
        let coordinator = ConversationTurnCoordinator::new();
        let mut config = LoongClawConfig::default();
        let memory_config = sqlite_memory_config("approval-control-bootstrap");
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        seed_pending_approval_request(&repo, "root-session", "apr-deny-1", "delegate_async", "app");

        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("root-session");
        let kernel_ctx =
            crate::context::bootstrap_test_kernel_context("approval-control-observer", 60)
                .expect("kernel context");

        let reply = coordinator
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "esc",
                ProviderErrorMode::Propagate,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
                Some(observer_handle),
            )
            .await
            .expect("approval control turn should succeed");

        assert_eq!(reply, "approval handled");

        let bootstrap_calls = runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap call lock should not be poisoned");
        assert_eq!(*bootstrap_calls, 1);

        let phase_events = observer
            .phase_events
            .lock()
            .expect("phase event lock should not be poisoned");
        let phase_names = phase_events
            .iter()
            .map(|event| event.phase)
            .collect::<Vec<_>>();

        assert_eq!(
            phase_names,
            vec![
                ConversationTurnPhase::Preparing,
                ConversationTurnPhase::ContextReady,
                ConversationTurnPhase::RunningTools,
                ConversationTurnPhase::FinalizingReply,
                ConversationTurnPhase::Completed,
            ]
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn pending_approval_control_turn_does_not_persist_session_mode_when_resolution_fails() {
        let coordinator = ConversationTurnCoordinator::new();
        let runtime = ApprovalControlRuntime::default();
        let mut config = LoongClawConfig::default();
        let memory_config = sqlite_memory_config("approval-control-session-mode");
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        seed_pending_approval_request(
            &repo,
            "root-session",
            "apr-auto-failure",
            "delegate_async",
            "app",
        );

        let db_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .to_path_buf();
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite connection");
        conn.execute_batch(
            "
            CREATE TRIGGER fail_pending_approval_update
            BEFORE UPDATE ON approval_requests
            BEGIN
                SELECT RAISE(FAIL, 'forced approval request update failure');
            END;
            ",
        )
        .expect("create approval failure trigger");

        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id("root-session");
        let result = coordinator
            .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
                &config,
                &address,
                "auto",
                ProviderErrorMode::Propagate,
                &runtime,
                &acp_options,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;
        assert!(
            result.is_ok(),
            "approval control turn should stay user-visible"
        );

        let stored = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent");
        assert!(
            stored.is_none(),
            "session mode should not persist on failure"
        );

        let approval_request = repo
            .load_approval_request("apr-auto-failure")
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(approval_request.status, ApprovalRequestStatus::Pending);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_persists_session_mode_on_success() {
        let runtime = ApprovalControlRuntime::default();
        let mut config = LoongClawConfig::default();
        let memory_config = sqlite_memory_config("approval-control-session-mode-success");
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        seed_pending_approval_request(
            &repo,
            "root-session",
            "apr-auto-success",
            "sessions_list",
            "app",
        );
        let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
        let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
            &config,
            &runtime,
            &fallback,
            ConversationRuntimeBinding::direct(),
        );
        let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "approval_request_resolve".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-auto-success",
                    "decision": "approve_once",
                    "session_consent_mode": "auto",
                }),
            },
            "root-session",
            &memory_config,
            &ToolConfig::default(),
            Some(&approval_runtime),
        )
        .await
        .expect("approval request resolve should succeed");
        assert_eq!(outcome.payload["approval_request"]["status"], "approved");

        let stored = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent")
            .expect("session tool consent row");
        assert_eq!(stored.mode, ToolConsentMode::Auto);
        assert_eq!(
            stored.updated_by_session_id.as_deref(),
            Some("root-session")
        );

        let approval_request = repo
            .load_approval_request("apr-auto-success")
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(
            approval_request.decision,
            Some(ApprovalDecision::ApproveOnce)
        );
        assert_eq!(approval_request.status, ApprovalRequestStatus::Approved);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn approval_request_resolve_retries_missing_session_mode_after_approval() {
        let runtime = ApprovalControlRuntime::default();
        let mut config = LoongClawConfig::default();
        let memory_config = sqlite_memory_config("approval-control-session-mode-retry");
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        seed_pending_approval_request(
            &repo,
            "root-session",
            "apr-auto-retry",
            "sessions_list",
            "app",
        );
        repo.transition_approval_request_if_current(
            "apr-auto-retry",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Approved,
                decision: Some(ApprovalDecision::ApproveOnce),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("transition approval request")
        .expect("approval request should be pending");

        let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
        let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
            &config,
            &runtime,
            &fallback,
            ConversationRuntimeBinding::direct(),
        );
        let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "approval_request_resolve".to_owned(),
                payload: json!({
                    "approval_request_id": "apr-auto-retry",
                    "decision": "approve_once",
                    "session_consent_mode": "auto",
                }),
            },
            "root-session",
            &memory_config,
            &ToolConfig::default(),
            Some(&approval_runtime),
        )
        .await
        .expect("approval request retry should succeed");

        assert_eq!(outcome.payload["approval_request"]["status"], "approved");

        let stored = repo
            .load_session_tool_consent("root-session")
            .expect("load session tool consent")
            .expect("session tool consent row");
        assert_eq!(stored.mode, ToolConsentMode::Auto);
        assert_eq!(
            stored.updated_by_session_id.as_deref(),
            Some("root-session")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn core_approval_replay_skips_app_session_context_loading() {
        let runtime = CoreReplayRuntime;
        let mut config = LoongClawConfig::default();
        let memory_config = sqlite_memory_config("approval-core-replay");
        let sqlite_path = memory_config
            .sqlite_path
            .as_ref()
            .expect("sqlite path")
            .display()
            .to_string();
        config.memory.sqlite_path = sqlite_path;
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
            approval_request_id: "apr-core-replay".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-core-replay".to_owned(),
            tool_call_id: "call-core-replay".to_owned(),
            tool_name: "provider.switch".to_owned(),
            approval_key: "tool:provider.switch".to_owned(),
            request_payload_json: json!({
                "session_id": "root-session",
                "turn_id": "turn-core-replay",
                "tool_call_id": "call-core-replay",
                "tool_name": "provider.switch",
                "args_json": {
                    "selector": "openai"
                },
                "source": "test",
                "execution_kind": "core",
            }),
            governance_snapshot_json: json!({
                "rule_id": "session_tool_consent_auto_blocked",
            }),
        })
        .expect("seed core approval request");
        let approval_request = repo
            .load_approval_request("apr-core-replay")
            .expect("load approval request")
            .expect("approval request row");
        let kernel_ctx =
            bootstrap_test_kernel_context("approval-core-replay", 60).expect("kernel context");
        let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
        let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
            &config,
            &runtime,
            &fallback,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        );

        let error = approval_runtime
            .replay_approved_request(&approval_request)
            .await
            .expect_err("provider.switch should fail at core execution");
        assert!(
            error.contains("provider.switch requires a resolved runtime config path"),
            "expected core execution failure, got: {error}"
        );
    }

    #[test]
    fn provider_turn_session_state_appends_user_input_and_keeps_estimate() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext {
                messages: vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })],
                artifacts: vec![],
                estimated_tokens: Some(42),
                prompt_fragments: Vec::new(),
                system_prompt_addition: None,
            },
            "hello world",
            None,
        );

        assert_eq!(session.estimated_tokens, Some(42));
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1]["role"], "user");
        assert_eq!(session.messages[1]["content"], "hello world");
        assert_eq!(
            session.prompt_frame_summary().turn_ephemeral_segment_count,
            1
        );
        assert!(
            session
                .prompt_frame_summary()
                .turn_ephemeral_estimated_tokens
                > 0
        );
    }

    #[test]
    fn provider_turn_session_state_after_turn_messages_appends_reply() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "hello world",
            None,
        );

        let messages = session.after_turn_messages("done");
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "done");
    }

    #[test]
    fn provider_turn_reply_tail_phase_captures_reply_and_after_turn_context() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext {
                messages: vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })],
                artifacts: vec![],
                estimated_tokens: Some(42),
                prompt_fragments: Vec::new(),
                system_prompt_addition: None,
            },
            "hello world",
            None,
        );

        let phase = ProviderTurnReplyTailPhase::from_session(&session, "done");

        assert_eq!(phase.reply(), "done");
        assert_eq!(phase.estimated_tokens(), Some(42));
        assert_eq!(phase.after_turn_messages().len(), 3);
        assert_eq!(phase.after_turn_messages()[2]["role"], "assistant");
        assert_eq!(phase.after_turn_messages()[2]["content"], "done");
    }

    #[test]
    fn provider_turn_followup_preparation_preserves_stable_prefix_hash_and_updates_tail_hash() {
        let base_fragment = crate::conversation::PromptFragment::new(
            "base-system",
            crate::conversation::PromptLane::BaseSystem,
            "base-system",
            "base system prompt",
            crate::conversation::ContextArtifactKind::SystemPrompt,
        )
        .with_cacheable(true);
        let assembled = AssembledConversationContext {
            messages: vec![
                serde_json::json!({
                    "role": "system",
                    "content": "base system prompt"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "recent assistant turn"
                }),
            ],
            artifacts: vec![
                crate::conversation::ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: crate::conversation::ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
                },
                crate::conversation::ContextArtifactDescriptor {
                    message_index: 1,
                    artifact_kind: crate::conversation::ContextArtifactKind::ConversationTurn,
                    maskable: false,
                    streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: Some(24),
            prompt_fragments: vec![base_fragment],
            system_prompt_addition: None,
        };
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &LoongClawConfig::default(),
            assembled,
            "use the recent result",
            None,
        );
        let mut followup_messages = preparation.session.messages.clone();

        followup_messages.push(serde_json::json!({
            "role": "assistant",
            "content": "[tool_result] compacted tail"
        }));
        followup_messages.push(serde_json::json!({
            "role": "user",
            "content": "continue"
        }));

        let followup_preparation = preparation.for_followup_messages(followup_messages);
        let initial_summary = preparation.session.prompt_frame_summary();
        let followup_summary = followup_preparation.session.prompt_frame_summary();

        assert_eq!(
            initial_summary.stable_prefix_hash_sha256,
            followup_summary.stable_prefix_hash_sha256
        );
        assert_eq!(
            initial_summary.cached_prefix_sha256,
            followup_summary.cached_prefix_sha256
        );
        assert_ne!(
            initial_summary.turn_ephemeral_hash_sha256,
            followup_summary.turn_ephemeral_hash_sha256
        );
        assert!(
            followup_summary.turn_ephemeral_segment_count
                > initial_summary.turn_ephemeral_segment_count
        );
    }

    #[test]
    fn provider_turn_followup_preparation_retains_original_tail_across_multiple_followups() {
        let base_fragment = crate::conversation::PromptFragment::new(
            "base-system",
            crate::conversation::PromptLane::BaseSystem,
            "base-system",
            "base system prompt",
            crate::conversation::ContextArtifactKind::SystemPrompt,
        )
        .with_cacheable(true);
        let assembled = AssembledConversationContext {
            messages: vec![
                serde_json::json!({
                    "role": "system",
                    "content": "base system prompt"
                }),
                serde_json::json!({
                    "role": "assistant",
                    "content": "recent assistant turn"
                }),
            ],
            artifacts: vec![
                crate::conversation::ContextArtifactDescriptor {
                    message_index: 0,
                    artifact_kind: crate::conversation::ContextArtifactKind::SystemPrompt,
                    maskable: false,
                    streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
                },
                crate::conversation::ContextArtifactDescriptor {
                    message_index: 1,
                    artifact_kind: crate::conversation::ContextArtifactKind::ConversationTurn,
                    maskable: false,
                    streaming_policy: crate::conversation::ToolOutputStreamingPolicy::BufferFull,
                },
            ],
            estimated_tokens: Some(24),
            prompt_fragments: vec![base_fragment],
            system_prompt_addition: None,
        };
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &LoongClawConfig::default(),
            assembled,
            "use the recent result",
            None,
        );
        let mut first_followup_messages = preparation.session.messages.clone();

        first_followup_messages.push(serde_json::json!({
            "role": "assistant",
            "content": "[tool_result] compacted tail"
        }));
        first_followup_messages.push(serde_json::json!({
            "role": "user",
            "content": "continue"
        }));

        let first_followup_preparation = preparation.for_followup_messages(first_followup_messages);
        let mut second_followup_messages = first_followup_preparation.session.messages.clone();

        second_followup_messages.push(serde_json::json!({
            "role": "assistant",
            "content": "[tool_result] second tail"
        }));
        second_followup_messages.push(serde_json::json!({
            "role": "user",
            "content": "continue again"
        }));

        let second_followup_preparation =
            first_followup_preparation.for_followup_messages(second_followup_messages);
        let second_summary = second_followup_preparation.session.prompt_frame_summary();
        let tail_bucket = second_summary
            .bucket(crate::conversation::PromptFrameLayer::TurnEphemeralTail)
            .expect("turn-ephemeral bucket should exist");

        assert_eq!(tail_bucket.message_count, 5);
        assert_eq!(second_summary.turn_ephemeral_segment_count, 5);
        assert_eq!(
            second_summary.stable_prefix_hash_sha256,
            preparation
                .session
                .prompt_frame_summary()
                .stable_prefix_hash_sha256
        );
    }

    #[test]
    fn provider_turn_lane_plan_hybrid_disabled_forces_fast_lane_limits() {
        let mut config = LoongClawConfig::default();
        config.conversation.hybrid_lane_enabled = false;
        config.conversation.fast_lane_max_tool_steps_per_turn = 3;
        config.conversation.safe_lane_max_tool_steps_per_turn = 7;

        let plan = ProviderTurnLanePlan::from_user_input(&config, "deploy to production");

        assert_eq!(plan.decision.lane, ExecutionLane::Fast);
        assert_eq!(plan.max_tool_steps, 3);
        assert!(
            plan.decision
                .reasons
                .iter()
                .any(|reason| reason.contains("hybrid_lane_disabled"))
        );
    }

    #[test]
    fn provider_turn_preparation_derives_lane_plan_and_raw_mode() {
        let mut config = LoongClawConfig::default();
        config.conversation.fast_lane_max_tool_steps_per_turn = 2;
        config.conversation.safe_lane_max_tool_steps_per_turn = 5;

        let preparation = ProviderTurnPreparation::from_assembled_context(
            &config,
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "deploy to production and show raw tool output",
            None,
        );

        assert_eq!(preparation.session.messages.len(), 2);
        assert_eq!(preparation.session.messages[1]["role"], "user");
        assert_eq!(
            preparation.session.messages[1]["content"],
            "deploy to production and show raw tool output"
        );
        assert!(preparation.raw_tool_output_requested);
        assert_eq!(preparation.lane_plan.decision.lane, ExecutionLane::Safe);
        assert_eq!(preparation.lane_plan.max_tool_steps, 5);
    }

    #[test]
    fn provider_turn_lane_plan_safe_plan_path_requires_safe_lane_and_tool_intents() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_plan_execution_enabled = true;

        let safe_plan = ProviderTurnLanePlan::from_user_input(
            &config,
            "deploy to production and rotate the token",
        );
        let tool_turn = ProviderTurn {
            assistant_text: "preface".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "echo hi"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe".to_owned(),
                turn_id: "turn-safe".to_owned(),
                tool_call_id: "call-safe".to_owned(),
            }],
            raw_meta: Value::Null,
        };

        assert_eq!(safe_plan.decision.lane, ExecutionLane::Safe);
        assert!(safe_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
        assert!(!safe_plan.should_use_safe_lane_plan_path(
            &config,
            &ProviderTurn {
                tool_intents: Vec::new(),
                ..tool_turn.clone()
            }
        ));

        let fast_plan = ProviderTurnLanePlan::from_user_input(&config, "say hello");
        assert_eq!(fast_plan.decision.lane, ExecutionLane::Fast);
        assert!(!fast_plan.should_use_safe_lane_plan_path(&config, &tool_turn));
    }

    #[test]
    fn provider_turn_continue_phase_checkpoint_captures_continue_branch_kernel_shape() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_max_tool_steps_per_turn = 5;
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &config,
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "deploy to production",
            None,
        );
        let phase = ProviderTurnContinuePhase::new(
            2,
            ProviderTurnLaneExecution {
                lane: ExecutionLane::Safe,
                assistant_preface: "preface".to_owned(),
                provider_usage: None,
                had_tool_intents: true,
                tool_request_summary: None,
                discovery_search_turn: false,
                search_tool_intents: 0,
                supports_provider_turn_followup: false,
                raw_tool_output_requested: false,
                turn_result: TurnResult::ToolError(TurnFailure::retryable(
                    "safe_lane_plan_node_retryable_error",
                    "transient",
                )),
                safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                    decision: SafeLaneFailureRouteDecision::Terminal,
                    reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                    source: SafeLaneFailureRouteSource::SessionGovernor,
                }),
                tool_events: Vec::new(),
            },
            None,
            config,
            None,
        );

        let checkpoint =
            phase.checkpoint(&preparation, "deploy to production", "preface\ntransient");

        assert_eq!(
            checkpoint.request,
            TurnCheckpointRequest::Continue { tool_intents: 2 }
        );
        assert_eq!(
            checkpoint
                .lane
                .as_ref()
                .expect("lane snapshot should be present")
                .result_kind,
            TurnCheckpointResultKind::ToolError
        );
        assert_eq!(
            checkpoint
                .lane
                .as_ref()
                .and_then(|lane| lane.safe_lane_terminal_route)
                .expect("safe-lane route should be present")
                .source,
            SafeLaneFailureRouteSource::SessionGovernor
        );
        assert_eq!(
            checkpoint
                .reply
                .as_ref()
                .expect("reply checkpoint should be present")
                .decision,
            ReplyResolutionMode::CompletionPass
        );
        assert_eq!(
            checkpoint
                .reply
                .as_ref()
                .and_then(|reply| reply.followup_kind),
            Some(ToolDrivenFollowupKind::ToolFailure)
        );
        assert_eq!(
            checkpoint.finalization,
            TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::Success,
                runs_after_turn: true,
                attempts_context_compaction: true,
            }
        );
        assert_eq!(
            checkpoint
                .identity
                .as_ref()
                .expect("identity should be present")
                .assistant_reply_chars,
            "preface\ntransient".chars().count()
        );
    }

    #[test]
    fn scope_provider_turn_tool_intents_overrides_existing_provider_ids_with_runtime_scope() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "tool.search".to_owned(),
                    args_json: json!({"query": "read file"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: String::new(),
                    turn_id: String::new(),
                    tool_call_id: "call-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "tool.invoke".to_owned(),
                    args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "already-session".to_owned(),
                    turn_id: "already-turn".to_owned(),
                    tool_call_id: "call-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        };

        let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

        // Provider-originated intents always get runtime scope overridden.
        assert_eq!(scoped.tool_intents[0].session_id, "session-a");
        assert_eq!(scoped.tool_intents[0].turn_id, "turn-a");
        assert_eq!(scoped.tool_intents[1].session_id, "session-a");
        assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
    }

    #[test]
    fn scope_non_provider_turn_tool_intents_preserve_existing_ids() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "tool.search".to_owned(),
                    args_json: json!({"query": "read file"}),
                    source: "local_followup".to_owned(),
                    session_id: "existing-session".to_owned(),
                    turn_id: "existing-turn".to_owned(),
                    tool_call_id: "call-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "tool.invoke".to_owned(),
                    args_json: json!({"tool_id": "file.read", "lease": "stub", "arguments": {"path": "README.md"}}),
                    source: "local_followup".to_owned(),
                    session_id: String::new(),
                    turn_id: String::new(),
                    tool_call_id: "call-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        };

        let scoped = scope_provider_turn_tool_intents(turn, "session-a", "turn-a");

        assert_eq!(scoped.tool_intents[0].session_id, "existing-session");
        assert_eq!(scoped.tool_intents[0].turn_id, "existing-turn");
        assert_eq!(scoped.tool_intents[1].session_id, "session-a");
        assert_eq!(scoped.tool_intents[1].turn_id, "turn-a");
    }

    #[test]
    fn reload_followup_provider_config_reads_provider_switch_wrapped_by_tool_invoke() {
        use std::fs;

        let root = std::env::temp_dir().join(format!(
            "loongclaw-provider-switch-followup-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let config_path = root.join("loongclaw.toml");

        let mut expected = LoongClawConfig::default();
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        expected.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai.clone(),
            },
        );
        expected.provider = openai;
        expected.active_provider = Some("openai-gpt-5".to_owned());
        fs::write(
            &config_path,
            crate::config::render(&expected).expect("render config"),
        )
        .expect("write config");

        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "tool.invoke".to_owned(),
                args_json: json!({
                    "tool_id": "provider.switch",
                    "lease": "ignored",
                    "arguments": {
                        "selector": "openai",
                        "config_path": config_path.to_string_lossy()
                    }
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            }],
            raw_meta: Value::Null,
        };

        let reloaded = ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(
            &LoongClawConfig::default(),
            &turn,
        );

        assert_eq!(reloaded.active_provider_id(), Some("openai-gpt-5"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provider_turn_continue_phase_checkpoint_keeps_direct_reply_without_followup() {
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &LoongClawConfig::default(),
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "say hello",
            None,
        );
        let phase = ProviderTurnContinuePhase::new(
            0,
            ProviderTurnLaneExecution {
                lane: ExecutionLane::Fast,
                assistant_preface: "preface".to_owned(),
                provider_usage: None,
                had_tool_intents: false,
                tool_request_summary: None,
                discovery_search_turn: false,
                search_tool_intents: 0,
                supports_provider_turn_followup: false,
                raw_tool_output_requested: false,
                turn_result: TurnResult::FinalText("hello there".to_owned()),
                safe_lane_terminal_route: None,
                tool_events: Vec::new(),
            },
            None,
            LoongClawConfig::default(),
            None,
        );

        let checkpoint = phase.checkpoint(&preparation, "say hello", "hello there");

        assert_eq!(
            checkpoint.request,
            TurnCheckpointRequest::Continue { tool_intents: 0 }
        );
        assert_eq!(
            checkpoint
                .lane
                .as_ref()
                .expect("lane snapshot should be present")
                .result_kind,
            TurnCheckpointResultKind::FinalText
        );
        assert_eq!(
            checkpoint
                .reply
                .as_ref()
                .expect("reply checkpoint should be present")
                .decision,
            ReplyResolutionMode::Direct
        );
        assert_eq!(
            checkpoint
                .reply
                .as_ref()
                .and_then(|reply| reply.followup_kind),
            None
        );
        assert_eq!(
            checkpoint
                .identity
                .as_ref()
                .expect("identity should be present")
                .assistant_reply_chars,
            "hello there".chars().count()
        );
    }

    #[test]
    fn resolved_provider_turn_checkpoint_preserves_safe_lane_route_provenance() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_max_tool_steps_per_turn = 5;

        let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
            reply: "preface\nsafe lane terminal".to_owned(),
            usage: None,
            checkpoint: TurnCheckpointSnapshot {
                identity: Some(TurnCheckpointIdentity::from_turn(
                    "deploy to production",
                    "preface\nsafe lane terminal",
                )),
                preparation: ProviderTurnPreparation::from_assembled_context(
                    &config,
                    AssembledConversationContext::from_messages(vec![serde_json::json!({
                        "role": "system",
                        "content": "sys"
                    })]),
                    "deploy to production",
                    None,
                )
                .checkpoint(),
                request: TurnCheckpointRequest::Continue { tool_intents: 1 },
                lane: Some(TurnLaneExecutionSnapshot {
                    lane: ExecutionLane::Safe,
                    had_tool_intents: true,
                    tool_request_summary: None,
                    raw_tool_output_requested: false,
                    result_kind: TurnCheckpointResultKind::ToolError,
                    safe_lane_terminal_route: Some(SafeLaneFailureRoute {
                        decision: SafeLaneFailureRouteDecision::Terminal,
                        reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                        source: SafeLaneFailureRouteSource::SessionGovernor,
                    }),
                }),
                reply: Some(TurnReplyCheckpoint {
                    decision: ReplyResolutionMode::CompletionPass,
                    followup_kind: Some(ToolDrivenFollowupKind::ToolFailure),
                }),
                finalization: TurnFinalizationCheckpoint::PersistReply {
                    persistence_mode: ReplyPersistenceMode::Success,
                    runs_after_turn: true,
                    attempts_context_compaction: true,
                },
            },
        });
        let snapshot = resolved.checkpoint();

        assert_eq!(snapshot.preparation.lane, ExecutionLane::Safe);
        assert_eq!(snapshot.preparation.context_message_count, 2);
        assert_eq!(
            snapshot.preparation.context_fingerprint_sha256,
            checkpoint_context_fingerprint_sha256(&[
                serde_json::json!({
                    "role": "system",
                    "content": "sys"
                }),
                serde_json::json!({
                    "role": "user",
                    "content": "deploy to production"
                }),
            ])
        );
        assert_eq!(
            snapshot.request,
            TurnCheckpointRequest::Continue { tool_intents: 1 }
        );
        assert_eq!(
            snapshot.lane.as_ref().expect("lane snapshot").result_kind,
            TurnCheckpointResultKind::ToolError
        );
        assert_eq!(
            snapshot
                .lane
                .as_ref()
                .and_then(|lane| lane.safe_lane_terminal_route)
                .expect("safe-lane route")
                .source,
            SafeLaneFailureRouteSource::SessionGovernor
        );
        assert_eq!(
            snapshot.reply.as_ref().expect("reply checkpoint").decision,
            ReplyResolutionMode::CompletionPass
        );
        assert_eq!(
            snapshot
                .reply
                .as_ref()
                .and_then(|reply| reply.followup_kind),
            Some(ToolDrivenFollowupKind::ToolFailure)
        );
        assert_eq!(
            snapshot.finalization,
            TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::Success,
                runs_after_turn: true,
                attempts_context_compaction: true,
            }
        );
        assert_eq!(
            snapshot
                .identity
                .as_ref()
                .expect("identity should be present")
                .user_input_chars,
            "deploy to production".chars().count()
        );
        assert_eq!(resolved.reply_text(), Some("preface\nsafe lane terminal"));
    }

    #[test]
    fn resolved_provider_turn_checkpoint_keeps_inline_provider_error_terminal_shape() {
        let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
            reply: "provider unavailable".to_owned(),
            usage: None,
            checkpoint: TurnCheckpointSnapshot {
                identity: Some(TurnCheckpointIdentity::from_turn(
                    "say hello",
                    "provider unavailable",
                )),
                preparation: ProviderTurnPreparation::from_assembled_context(
                    &LoongClawConfig::default(),
                    AssembledConversationContext::from_messages(vec![serde_json::json!({
                        "role": "system",
                        "content": "sys"
                    })]),
                    "say hello",
                    None,
                )
                .checkpoint(),
                request: TurnCheckpointRequest::FinalizeInlineProviderError,
                lane: None,
                reply: None,
                finalization: TurnFinalizationCheckpoint::PersistReply {
                    persistence_mode: ReplyPersistenceMode::InlineProviderError,
                    runs_after_turn: true,
                    attempts_context_compaction: true,
                },
            },
        });
        let snapshot = resolved.checkpoint();

        assert_eq!(
            snapshot.request,
            TurnCheckpointRequest::FinalizeInlineProviderError
        );
        assert!(snapshot.lane.is_none());
        assert!(snapshot.reply.is_none());
        assert!(snapshot.identity.is_some());
        assert_eq!(
            snapshot.finalization,
            TurnFinalizationCheckpoint::PersistReply {
                persistence_mode: ReplyPersistenceMode::InlineProviderError,
                runs_after_turn: true,
                attempts_context_compaction: true,
            }
        );
        assert_eq!(resolved.reply_text(), Some("provider unavailable"));
    }

    #[test]
    fn resolved_provider_turn_checkpoint_marks_return_error_finalization() {
        let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
            error: "provider unavailable".to_owned(),
            checkpoint: TurnCheckpointSnapshot {
                identity: None,
                preparation: ProviderTurnPreparation::from_assembled_context(
                    &LoongClawConfig::default(),
                    AssembledConversationContext::from_messages(vec![serde_json::json!({
                        "role": "system",
                        "content": "sys"
                    })]),
                    "say hello",
                    None,
                )
                .checkpoint(),
                request: TurnCheckpointRequest::ReturnError,
                lane: None,
                reply: None,
                finalization: TurnFinalizationCheckpoint::ReturnError,
            },
        });
        let snapshot = resolved.checkpoint();

        assert_eq!(snapshot.request, TurnCheckpointRequest::ReturnError);
        assert!(snapshot.identity.is_none());
        assert!(snapshot.lane.is_none());
        assert!(snapshot.reply.is_none());
        assert_eq!(
            snapshot.finalization,
            TurnFinalizationCheckpoint::ReturnError
        );
        assert_eq!(resolved.reply_text(), None);
    }

    #[test]
    fn resolved_provider_turn_terminal_phase_builds_reply_tail_and_checkpoint() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext {
                messages: vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })],
                artifacts: vec![],
                estimated_tokens: Some(42),
                prompt_fragments: Vec::new(),
                system_prompt_addition: None,
            },
            "say hello",
            None,
        );
        let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
            reply: "done".to_owned(),
            usage: None,
            checkpoint: TurnCheckpointSnapshot {
                identity: Some(TurnCheckpointIdentity::from_turn("say hello", "done")),
                preparation: ProviderTurnPreparation::from_assembled_context(
                    &LoongClawConfig::default(),
                    AssembledConversationContext::from_messages(vec![serde_json::json!({
                        "role": "system",
                        "content": "sys"
                    })]),
                    "say hello",
                    None,
                )
                .checkpoint(),
                request: TurnCheckpointRequest::Continue { tool_intents: 0 },
                lane: Some(TurnLaneExecutionSnapshot {
                    lane: ExecutionLane::Fast,
                    had_tool_intents: false,
                    tool_request_summary: None,
                    raw_tool_output_requested: false,
                    result_kind: TurnCheckpointResultKind::FinalText,
                    safe_lane_terminal_route: None,
                }),
                reply: Some(TurnReplyCheckpoint {
                    decision: ReplyResolutionMode::Direct,
                    followup_kind: None,
                }),
                finalization: TurnFinalizationCheckpoint::persist_reply(
                    ReplyPersistenceMode::Success,
                ),
            },
        });

        let phase = resolved.terminal_phase(&session);

        match phase {
            ProviderTurnTerminalPhase::PersistReply(phase) => {
                assert_eq!(
                    phase.checkpoint.request,
                    TurnCheckpointRequest::Continue { tool_intents: 0 }
                );
                assert_eq!(phase.tail_phase.reply(), "done");
                assert_eq!(phase.tail_phase.estimated_tokens(), Some(42));
                assert_eq!(phase.tail_phase.after_turn_messages().len(), 3);
                assert_eq!(phase.tail_phase.after_turn_messages()[2]["content"], "done");
            }
            ProviderTurnTerminalPhase::ReturnError(_) => {
                panic!("persist reply should build persist terminal phase")
            }
        }
    }

    #[test]
    fn resolved_provider_turn_terminal_phase_preserves_return_error_checkpoint() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "say hello",
            None,
        );
        let resolved = ResolvedProviderTurn::ReturnError(ResolvedProviderError {
            error: "provider unavailable".to_owned(),
            checkpoint: TurnCheckpointSnapshot {
                identity: None,
                preparation: ProviderTurnPreparation::from_assembled_context(
                    &LoongClawConfig::default(),
                    AssembledConversationContext::from_messages(vec![serde_json::json!({
                        "role": "system",
                        "content": "sys"
                    })]),
                    "say hello",
                    None,
                )
                .checkpoint(),
                request: TurnCheckpointRequest::ReturnError,
                lane: None,
                reply: None,
                finalization: TurnFinalizationCheckpoint::ReturnError,
            },
        });

        let phase = resolved.terminal_phase(&session);

        match phase {
            ProviderTurnTerminalPhase::ReturnError(phase) => {
                assert_eq!(phase.checkpoint.request, TurnCheckpointRequest::ReturnError);
                assert_eq!(phase.error, "provider unavailable");
            }
            ProviderTurnTerminalPhase::PersistReply(_) => {
                panic!("return error should build return-error terminal phase")
            }
        }
    }

    #[test]
    fn provider_turn_request_terminal_phase_builds_inline_provider_error_reply() {
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &LoongClawConfig::default(),
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "say hello",
            None,
        );

        let resolved = ProviderTurnRequestTerminalPhase::persist_inline_provider_error(
            "provider unavailable".to_owned(),
        )
        .resolve(&preparation, "say hello");

        match resolved {
            ResolvedProviderTurn::PersistReply(reply) => {
                assert_eq!(reply.reply, "provider unavailable");
                assert_eq!(
                    reply.checkpoint.request,
                    TurnCheckpointRequest::FinalizeInlineProviderError
                );
                assert!(reply.checkpoint.lane.is_none());
                assert!(reply.checkpoint.reply.is_none());
                assert_eq!(
                    reply.checkpoint.finalization,
                    TurnFinalizationCheckpoint::persist_reply(
                        ReplyPersistenceMode::InlineProviderError,
                    )
                );
                assert!(reply.checkpoint.identity.is_some());
            }
            ResolvedProviderTurn::ReturnError(_) => {
                panic!("inline provider error should resolve to persisted reply")
            }
        }
    }

    #[test]
    fn provider_turn_request_terminal_phase_builds_return_error_without_reply_identity() {
        let preparation = ProviderTurnPreparation::from_assembled_context(
            &LoongClawConfig::default(),
            AssembledConversationContext::from_messages(vec![serde_json::json!({
                "role": "system",
                "content": "sys"
            })]),
            "say hello",
            None,
        );

        let resolved =
            ProviderTurnRequestTerminalPhase::return_error("provider unavailable".to_owned())
                .resolve(&preparation, "say hello");

        match resolved {
            ResolvedProviderTurn::ReturnError(error) => {
                assert_eq!(error.error, "provider unavailable");
                assert_eq!(error.checkpoint.request, TurnCheckpointRequest::ReturnError);
                assert!(error.checkpoint.identity.is_none());
                assert!(error.checkpoint.lane.is_none());
                assert!(error.checkpoint.reply.is_none());
                assert_eq!(
                    error.checkpoint.finalization,
                    TurnFinalizationCheckpoint::ReturnError
                );
            }
            ResolvedProviderTurn::PersistReply(_) => {
                panic!("propagated provider error should resolve to return-error outcome")
            }
        }
    }

    #[test]
    fn safe_lane_replan_budget_allows_one_retry_then_exhausts() {
        let initial = SafeLaneReplanBudget::new(1);

        assert_eq!(
            initial.continuation_decision(),
            SafeLaneContinuationBudgetDecision::Continue
        );
        assert_eq!(initial.current_round(), 0);

        let exhausted = initial.after_replan();
        assert_eq!(
            exhausted.continuation_decision(),
            SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            }
        );
        assert_eq!(exhausted.current_round(), 1);
    }

    #[test]
    fn escalating_attempt_budget_caps_growth_at_maximum() {
        let budget = EscalatingAttemptBudget::new(2, 4);

        assert_eq!(budget.current_limit(), 2);
        assert_eq!(budget.after_retry().current_limit(), 3);
        assert_eq!(budget.after_retry().after_retry().current_limit(), 4);
        assert_eq!(
            budget
                .after_retry()
                .after_retry()
                .after_retry()
                .current_limit(),
            4
        );
    }

    #[test]
    fn decide_provider_request_action_continues_on_success() {
        let decision = decide_provider_turn_request_action(
            Ok(ProviderTurn {
                assistant_text: "preface".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            }),
            ProviderErrorMode::Propagate,
        );

        if let ProviderTurnRequestAction::Continue { turn } = decision {
            assert_eq!(turn.assistant_text, "preface");
            assert!(turn.tool_intents.is_empty());
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[path = "turn_coordinator_safe_lane_route_tests.rs"]
    mod safe_lane_route_tests;
}
