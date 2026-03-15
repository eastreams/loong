#[cfg(feature = "memory-sqlite")]
use std::any::Any;
use std::collections::BTreeSet;
#[cfg(feature = "memory-sqlite")]
use std::panic::AssertUnwindSafe;
#[cfg(feature = "memory-sqlite")]
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use futures_util::FutureExt;
use loongclaw_contracts::{AuditEventKind, ExecutionPlane, PlaneTier};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[cfg(feature = "memory-sqlite")]
use tokio::runtime::Handle;
use tokio::sync::Mutex;
#[cfg(feature = "memory-sqlite")]
use tokio::time::{Duration, Instant, timeout};

use crate::CliResult;
use crate::KernelContext;
use crate::acp::{
    AcpConversationTurnEntryDecision, AcpConversationTurnExecutionOutcome,
    AcpConversationTurnOptions, AcpTurnEventSink, evaluate_acp_conversation_turn_entry_for_address,
    execute_acp_conversation_turn_for_address,
};
use crate::memory::runtime_config::MemoryRuntimeConfig;

use super::super::config::LoongClawConfig;
use super::ConversationSessionAddress;
use super::ProviderErrorMode;
use super::analytics::{
    SafeLaneEventSummary, TurnCheckpointProgressStatus as AnalyticsTurnCheckpointProgressStatus,
    TurnCheckpointRecoveryAction, TurnCheckpointRepairManualReason, TurnCheckpointRepairPlan,
    TurnCheckpointSessionState, build_turn_checkpoint_repair_plan, summarize_safe_lane_history,
};
use super::context_engine::AssembledConversationContext;
use super::ingress::ConversationIngressContext;
use super::lane_arbiter::{ExecutionLane, LaneArbiterPolicy, LaneDecision};
use super::persistence::{
    format_provider_error_reply, persist_acp_runtime_events, persist_conversation_event,
    persist_reply_turns_raw_with_mode, persist_reply_turns_with_mode,
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
    AsyncDelegateSpawnRequest, AsyncDelegateSpawner, ConversationRuntime,
    DefaultConversationRuntime, SessionContext,
};
use super::safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    classify_safe_lane_plan_failure,
};
#[cfg(feature = "memory-sqlite")]
use super::session_history::{
    load_assistant_contents_from_session_window, load_latest_turn_checkpoint_entry,
    load_turn_checkpoint_history_snapshot,
};
use super::turn_budget::{
    EscalatingAttemptBudget, SafeLaneBackpressureBudget, SafeLaneContinuationBudgetDecision,
    SafeLaneFailureRouteReason, SafeLaneReplanBudget,
};
use super::turn_engine::{
    AppToolDispatcher, DefaultAppToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnFailure,
    TurnFailureKind, TurnResult, TurnValidation,
};
use super::turn_shared::{
    ProviderTurnRequestAction, ReplyPersistenceMode, ReplyResolutionMode, ToolDrivenFollowupKind,
    ToolDrivenFollowupPayload, ToolDrivenReplyBaseDecision, ToolDrivenReplyPhase,
    build_tool_driven_followup_tail, decide_provider_turn_request_action,
    format_approval_required_reply, request_completion_with_raw_fallback,
    tool_result_contains_truncation_signal, user_requested_raw_tool_output,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{
    RECOVERY_EVENT_KIND, build_async_spawn_failure_recovery_payload,
    build_terminal_finalize_recovery_payload,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    ApprovalDecision, ApprovalRequestRecord, ApprovalRequestStatus, CreateSessionWithEventRequest,
    FinalizeSessionTerminalRequest, NewApprovalGrantRecord, NewSessionRecord, SessionKind,
    SessionRepository, SessionState, TransitionApprovalRequestIfCurrentRequest,
    TransitionSessionWithEventIfCurrentRequest,
};

#[derive(Default)]
pub struct ConversationTurnCoordinator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairStatus {
    NoCheckpoint,
    NotNeeded,
    Repaired,
    ManualRequired,
}

impl TurnCheckpointTailRepairStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCheckpoint => "no_checkpoint",
            Self::NotNeeded => "not_needed",
            Self::Repaired => "repaired",
            Self::ManualRequired => "manual_required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairReason {
    NoCheckpoint,
    NotNeeded,
    Repaired,
    CheckpointIdentityMissing,
    SafeLaneBackpressureTerminalRequiresManualInspection,
    SafeLaneSessionGovernorTerminalRequiresManualInspection,
    CheckpointPreparationMalformed,
    CheckpointPreparationMismatch,
    CheckpointPreparationFingerprintMismatch,
    CheckpointStateRequiresManualInspection,
    VisibleTurnPairMissing,
    CheckpointIdentityMismatch,
}

impl TurnCheckpointTailRepairReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoCheckpoint => "no_checkpoint",
            Self::NotNeeded => "not_needed",
            Self::Repaired => "repaired",
            Self::CheckpointIdentityMissing => "checkpoint_identity_missing",
            Self::SafeLaneBackpressureTerminalRequiresManualInspection => {
                "safe_lane_backpressure_terminal_requires_manual_inspection"
            }
            Self::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                "safe_lane_session_governor_terminal_requires_manual_inspection"
            }
            Self::CheckpointPreparationMalformed => "checkpoint_preparation_malformed",
            Self::CheckpointPreparationMismatch => "checkpoint_preparation_mismatch",
            Self::CheckpointPreparationFingerprintMismatch => {
                "checkpoint_preparation_fingerprint_mismatch"
            }
            Self::CheckpointStateRequiresManualInspection => {
                "checkpoint_state_requires_manual_inspection"
            }
            Self::VisibleTurnPairMissing => "visible_turn_pair_missing",
            Self::CheckpointIdentityMismatch => "checkpoint_identity_mismatch",
        }
    }
}

impl From<TurnCheckpointRepairManualReason> for TurnCheckpointTailRepairReason {
    fn from(reason: TurnCheckpointRepairManualReason) -> Self {
        match reason {
            TurnCheckpointRepairManualReason::CheckpointIdentityMissing => {
                Self::CheckpointIdentityMissing
            }
            TurnCheckpointRepairManualReason::SafeLaneBackpressureTerminalRequiresManualInspection => {
                Self::SafeLaneBackpressureTerminalRequiresManualInspection
            }
            TurnCheckpointRepairManualReason::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                Self::SafeLaneSessionGovernorTerminalRequiresManualInspection
            }
            TurnCheckpointRepairManualReason::CheckpointStateRequiresManualInspection => {
                Self::CheckpointStateRequiresManualInspection
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCheckpointTailRepairOutcome {
    status: TurnCheckpointTailRepairStatus,
    action: TurnCheckpointRecoveryAction,
    source: Option<TurnCheckpointTailRepairSource>,
    reason: TurnCheckpointTailRepairReason,
    session_state: TurnCheckpointSessionState,
    checkpoint_events: u32,
    after_turn_status: Option<&'static str>,
    compaction_status: Option<&'static str>,
}

impl TurnCheckpointTailRepairOutcome {
    fn no_checkpoint() -> Self {
        Self {
            status: TurnCheckpointTailRepairStatus::NoCheckpoint,
            action: TurnCheckpointRecoveryAction::None,
            source: None,
            reason: TurnCheckpointTailRepairReason::NoCheckpoint,
            session_state: TurnCheckpointSessionState::NotDurable,
            checkpoint_events: 0,
            after_turn_status: None,
            compaction_status: None,
        }
    }

    pub(crate) fn from_summary(
        status: TurnCheckpointTailRepairStatus,
        action: TurnCheckpointRecoveryAction,
        source: Option<TurnCheckpointTailRepairSource>,
        reason: TurnCheckpointTailRepairReason,
        summary: &super::analytics::TurnCheckpointEventSummary,
    ) -> Self {
        Self {
            status,
            action,
            source,
            reason,
            session_state: summary.session_state,
            checkpoint_events: summary.checkpoint_events,
            after_turn_status: summary
                .latest_after_turn
                .map(format_analytics_turn_checkpoint_progress_status),
            compaction_status: summary
                .latest_compaction
                .map(format_analytics_turn_checkpoint_progress_status),
        }
    }

    fn repaired(
        action: TurnCheckpointRecoveryAction,
        summary: &super::analytics::TurnCheckpointEventSummary,
        after_turn_status: TurnCheckpointProgressStatus,
        compaction_status: TurnCheckpointProgressStatus,
    ) -> Self {
        Self {
            status: TurnCheckpointTailRepairStatus::Repaired,
            action,
            source: Some(TurnCheckpointTailRepairSource::Runtime),
            reason: TurnCheckpointTailRepairReason::Repaired,
            session_state: summary.session_state,
            checkpoint_events: summary.checkpoint_events,
            after_turn_status: Some(format_turn_checkpoint_progress_status(after_turn_status)),
            compaction_status: Some(format_turn_checkpoint_progress_status(compaction_status)),
        }
    }

    pub fn status(&self) -> TurnCheckpointTailRepairStatus {
        self.status
    }

    pub fn action(&self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(&self) -> Option<TurnCheckpointTailRepairSource> {
        self.source
    }

    pub fn reason(&self) -> TurnCheckpointTailRepairReason {
        self.reason
    }

    pub fn session_state(&self) -> TurnCheckpointSessionState {
        self.session_state
    }

    pub fn checkpoint_events(&self) -> u32 {
        self.checkpoint_events
    }

    pub fn after_turn_status(&self) -> Option<&'static str> {
        self.after_turn_status
    }

    pub fn compaction_status(&self) -> Option<&'static str> {
        self.compaction_status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointTailRepairSource {
    Summary,
    Runtime,
}

impl TurnCheckpointTailRepairSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnCheckpointTailRepairRuntimeProbe {
    action: TurnCheckpointRecoveryAction,
    source: TurnCheckpointTailRepairSource,
    reason: TurnCheckpointTailRepairReason,
}

impl TurnCheckpointTailRepairRuntimeProbe {
    pub(crate) fn new(
        action: TurnCheckpointRecoveryAction,
        source: TurnCheckpointTailRepairSource,
        reason: TurnCheckpointTailRepairReason,
    ) -> Self {
        Self {
            action,
            source,
            reason,
        }
    }

    pub fn action(&self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(&self) -> TurnCheckpointTailRepairSource {
        self.source
    }

    pub fn reason(&self) -> TurnCheckpointTailRepairReason {
        self.reason
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TurnCheckpointRecoveryAssessment {
    action: TurnCheckpointRecoveryAction,
    source: TurnCheckpointTailRepairSource,
    reason: Option<TurnCheckpointTailRepairReason>,
}

impl TurnCheckpointRecoveryAssessment {
    pub(crate) fn from_summary(summary: &super::analytics::TurnCheckpointEventSummary) -> Self {
        let repair_plan = build_turn_checkpoint_repair_plan(summary);
        let reason = matches!(
            repair_plan.action(),
            TurnCheckpointRecoveryAction::InspectManually
        )
        .then(|| {
            repair_plan
                .manual_reason()
                .map(TurnCheckpointTailRepairReason::from)
                .unwrap_or(TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection)
        });
        Self {
            action: repair_plan.action(),
            source: TurnCheckpointTailRepairSource::Summary,
            reason,
        }
    }

    pub fn action(self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn source(self) -> TurnCheckpointTailRepairSource {
        self.source
    }

    pub fn reason(self) -> Option<TurnCheckpointTailRepairReason> {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnCheckpointDiagnostics {
    summary: super::analytics::TurnCheckpointEventSummary,
    recovery: TurnCheckpointRecoveryAssessment,
    runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
}

impl TurnCheckpointDiagnostics {
    pub(crate) fn new(
        summary: super::analytics::TurnCheckpointEventSummary,
        recovery: TurnCheckpointRecoveryAssessment,
        runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
    ) -> Self {
        Self {
            summary,
            recovery,
            runtime_probe,
        }
    }

    pub fn summary(&self) -> &super::analytics::TurnCheckpointEventSummary {
        &self.summary
    }

    pub fn recovery(&self) -> TurnCheckpointRecoveryAssessment {
        self.recovery
    }

    pub fn runtime_probe(&self) -> Option<&TurnCheckpointTailRepairRuntimeProbe> {
        self.runtime_probe.as_ref()
    }
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
            replan_budget: SafeLaneReplanBudget::new(if governor.force_no_replan {
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
struct ProviderTurnSessionState {
    messages: Vec<Value>,
    estimated_tokens: Option<usize>,
}

impl ProviderTurnSessionState {
    fn from_assembled_context(
        assembled_context: AssembledConversationContext,
        user_input: &str,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let mut messages = assembled_context.messages;
        if let Some(ingress) = ingress.filter(|value| value.has_contextual_hints()) {
            messages.push(ingress.as_system_message());
        }
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        Self {
            messages,
            estimated_tokens: assembled_context.estimated_tokens,
        }
    }

    fn after_turn_messages(&self, reply: &str) -> Vec<Value> {
        let mut messages = self.messages.clone();
        messages.push(json!({
            "role": "assistant",
            "content": reply,
        }));
        messages
    }
}

#[derive(Debug, Clone)]
struct ProviderTurnReplyTailPhase {
    reply: String,
    after_turn_messages: Vec<Value>,
    estimated_tokens: Option<usize>,
}

impl ProviderTurnReplyTailPhase {
    fn from_session(session: &ProviderTurnSessionState, reply: &str) -> Self {
        Self {
            reply: reply.to_owned(),
            after_turn_messages: session.after_turn_messages(reply),
            estimated_tokens: session.estimated_tokens,
        }
    }

    fn reply(&self) -> &str {
        self.reply.as_str()
    }

    fn after_turn_messages(&self) -> &[Value] {
        &self.after_turn_messages
    }

    fn estimated_tokens(&self) -> Option<usize> {
        self.estimated_tokens
    }
}

#[derive(Debug, Clone)]
struct ProviderTurnPreparation {
    session: ProviderTurnSessionState,
    lane_plan: ProviderTurnLanePlan,
    raw_tool_output_requested: bool,
    turn_id: String,
}

impl ProviderTurnPreparation {
    fn from_assembled_context(
        config: &LoongClawConfig,
        assembled_context: AssembledConversationContext,
        user_input: &str,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let turn_id = build_provider_turn_scope_id(&assembled_context.messages, user_input);
        Self {
            session: ProviderTurnSessionState::from_assembled_context(
                assembled_context,
                user_input,
                ingress,
            ),
            lane_plan: ProviderTurnLanePlan::from_user_input(config, user_input),
            raw_tool_output_requested: user_requested_raw_tool_output(user_input),
            turn_id,
        }
    }

    fn checkpoint(&self) -> TurnPreparationSnapshot {
        TurnPreparationSnapshot {
            lane: self.lane_plan.decision.lane,
            max_tool_steps: self.lane_plan.max_tool_steps,
            raw_tool_output_requested: self.raw_tool_output_requested,
            context_message_count: self.session.messages.len(),
            context_fingerprint_sha256: checkpoint_context_fingerprint_sha256(
                &self.session.messages,
            ),
            estimated_tokens: self.session.estimated_tokens,
        }
    }
}

fn build_provider_turn_scope_id(messages: &[Value], user_input: &str) -> String {
    let mut visible_context = messages.to_vec();
    visible_context.push(json!({
        "role": "user",
        "content": user_input,
    }));
    format!(
        "turn:{}",
        checkpoint_context_fingerprint_sha256(&visible_context)
    )
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
    had_tool_intents: bool,
    raw_tool_output_requested: bool,
    turn_result: TurnResult,
    safe_lane_terminal_route: Option<SafeLaneFailureRoute>,
}

impl ProviderTurnLaneExecution {
    fn checkpoint(&self) -> TurnLaneExecutionSnapshot {
        TurnLaneExecutionSnapshot {
            lane: self.lane,
            had_tool_intents: self.had_tool_intents,
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

#[derive(Debug, Clone)]
struct ProviderTurnContinuePhase {
    request: TurnCheckpointRequest,
    lane_execution: ProviderTurnLaneExecution,
    reply_phase: ToolDrivenReplyPhase,
    followup_config: LoongClawConfig,
}

impl ProviderTurnContinuePhase {
    fn new(
        tool_intents: usize,
        lane_execution: ProviderTurnLaneExecution,
        followup_config: LoongClawConfig,
    ) -> Self {
        let reply_phase = lane_execution.reply_phase();
        Self {
            request: TurnCheckpointRequest::Continue { tool_intents },
            lane_execution,
            reply_phase,
            followup_config,
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
            Some(turn_reply_checkpoint(&self.reply_phase)),
            TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
        )
    }

    async fn resolve_reply<R: ConversationRuntime + ?Sized>(
        &self,
        runtime: &R,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> String {
        resolve_provider_turn_reply(
            runtime,
            &self.followup_config,
            preparation,
            &self.lane_execution,
            &self.reply_phase,
            user_input,
            kernel_ctx,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnCheckpointSnapshot {
    identity: Option<TurnCheckpointIdentity>,
    preparation: TurnPreparationSnapshot,
    request: TurnCheckpointRequest,
    lane: Option<TurnLaneExecutionSnapshot>,
    reply: Option<TurnReplyCheckpoint>,
    finalization: TurnFinalizationCheckpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TurnCheckpointIdentity {
    user_input_sha256: String,
    assistant_reply_sha256: String,
    user_input_chars: usize,
    assistant_reply_chars: usize,
}

impl TurnCheckpointIdentity {
    fn from_turn(user_input: &str, assistant_reply: &str) -> Self {
        Self {
            user_input_sha256: sha256_hex(user_input),
            assistant_reply_sha256: sha256_hex(assistant_reply),
            user_input_chars: user_input.chars().count(),
            assistant_reply_chars: assistant_reply.chars().count(),
        }
    }

    fn matches_turn(&self, user_input: &str, assistant_reply: &str) -> bool {
        self.user_input_chars == user_input.chars().count()
            && self.assistant_reply_chars == assistant_reply.chars().count()
            && self.user_input_sha256 == sha256_hex(user_input)
            && self.assistant_reply_sha256 == sha256_hex(assistant_reply)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnPreparationSnapshot {
    lane: ExecutionLane,
    max_tool_steps: usize,
    raw_tool_output_requested: bool,
    context_message_count: usize,
    context_fingerprint_sha256: String,
    estimated_tokens: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TurnCheckpointRequest {
    Continue { tool_intents: usize },
    FinalizeInlineProviderError,
    ReturnError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnLaneExecutionSnapshot {
    lane: ExecutionLane,
    had_tool_intents: bool,
    raw_tool_output_requested: bool,
    result_kind: TurnCheckpointResultKind,
    safe_lane_terminal_route: Option<SafeLaneFailureRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TurnCheckpointResultKind {
    FinalText,
    NeedsApproval,
    ToolDenied,
    ToolError,
    ProviderError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnReplyCheckpoint {
    decision: ReplyResolutionMode,
    followup_kind: Option<ToolDrivenFollowupKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TurnFinalizationCheckpoint {
    PersistReply {
        persistence_mode: ReplyPersistenceMode,
        runs_after_turn: bool,
        attempts_context_compaction: bool,
    },
    ReturnError,
}

impl TurnFinalizationCheckpoint {
    fn persist_reply(persistence_mode: ReplyPersistenceMode) -> Self {
        Self::PersistReply {
            persistence_mode,
            runs_after_turn: true,
            attempts_context_compaction: true,
        }
    }

    fn persistence_mode(self) -> Option<ReplyPersistenceMode> {
        match self {
            Self::PersistReply {
                persistence_mode, ..
            } => Some(persistence_mode),
            Self::ReturnError => None,
        }
    }

    fn runs_after_turn(self) -> bool {
        match self {
            Self::PersistReply {
                runs_after_turn, ..
            } => runs_after_turn,
            Self::ReturnError => false,
        }
    }

    fn attempts_context_compaction(self) -> bool {
        match self {
            Self::PersistReply {
                attempts_context_compaction,
                ..
            } => attempts_context_compaction,
            Self::ReturnError => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TurnCheckpointStage {
    PostPersist,
    Finalized,
    FinalizationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TurnCheckpointProgressStatus {
    Pending,
    Skipped,
    Completed,
    Failed,
    FailedOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct TurnCheckpointFinalizationProgress {
    after_turn: TurnCheckpointProgressStatus,
    compaction: TurnCheckpointProgressStatus,
}

impl TurnCheckpointFinalizationProgress {
    fn pending(checkpoint: &TurnCheckpointSnapshot) -> Self {
        Self {
            after_turn: if checkpoint.finalization.runs_after_turn() {
                TurnCheckpointProgressStatus::Pending
            } else {
                TurnCheckpointProgressStatus::Skipped
            },
            compaction: if checkpoint.finalization.attempts_context_compaction() {
                TurnCheckpointProgressStatus::Pending
            } else {
                TurnCheckpointProgressStatus::Skipped
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextCompactionOutcome {
    Skipped,
    Completed,
    FailedOpen,
}

impl ContextCompactionOutcome {
    fn checkpoint_status(self) -> TurnCheckpointProgressStatus {
        match self {
            Self::Skipped => TurnCheckpointProgressStatus::Skipped,
            Self::Completed => TurnCheckpointProgressStatus::Completed,
            Self::FailedOpen => TurnCheckpointProgressStatus::FailedOpen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TurnCheckpointFailureStep {
    AfterTurn,
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TurnCheckpointFailure {
    step: TurnCheckpointFailureStep,
    error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedProviderTurn {
    PersistReply(ResolvedProviderReply),
    ReturnError(ResolvedProviderError),
}

impl ResolvedProviderTurn {
    fn persist_reply(reply: String, checkpoint: TurnCheckpointSnapshot) -> Self {
        Self::PersistReply(ResolvedProviderReply { reply, checkpoint })
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProviderReply {
    reply: String,
    checkpoint: TurnCheckpointSnapshot,
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
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        match self {
            Self::PersistReply(phase) => {
                finalize_provider_turn_reply(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    &phase.tail_phase,
                    phase.checkpoint,
                    kernel_ctx,
                )
                .await
            }
            Self::ReturnError(phase) => {
                persist_resolved_provider_error_checkpoint(
                    runtime,
                    session_id,
                    phase.checkpoint,
                    kernel_ctx,
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
                ResolvedProviderTurn::persist_reply(reply, checkpoint)
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

fn turn_reply_checkpoint(phase: &ToolDrivenReplyPhase) -> TurnReplyCheckpoint {
    TurnReplyCheckpoint {
        decision: phase.resolution_mode(),
        followup_kind: phase.followup_kind(),
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

    pub async fn handle_turn_with_ingress(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        kernel_ctx: Option<&KernelContext>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let acp_options = AcpConversationTurnOptions::automatic();
        let address = ConversationSessionAddress::from_session_id(session_id);
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            &address,
            user_input,
            error_mode,
            &runtime,
            &acp_options,
            kernel_ctx,
            ingress,
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

    pub async fn repair_turn_checkpoint_tail(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.repair_turn_checkpoint_tail_with_runtime(config, session_id, &runtime, kernel_ctx)
            .await
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            &runtime,
            kernel_ctx,
        )
        .await
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config, session_id, limit, &runtime, kernel_ctx,
        )
        .await
    }

    pub async fn probe_turn_checkpoint_tail_runtime_gate(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            &runtime,
            kernel_ctx,
        )
        .await
    }

    pub async fn probe_turn_checkpoint_tail_runtime_gate_with_limit(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config, session_id, limit, &runtime, kernel_ctx,
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

    pub async fn handle_turn_with_address_and_acp_options_and_ingress(
        &self,
        config: &LoongClawConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        acp_options: &AcpConversationTurnOptions<'_>,
        kernel_ctx: Option<&KernelContext>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            kernel_ctx,
            ingress,
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
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            &runtime,
            acp_options,
            kernel_ctx,
            None,
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

    pub async fn handle_turn_with_runtime_and_ingress<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
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
            kernel_ctx,
            ingress,
        )
        .await
    }

    pub async fn repair_turn_checkpoint_tail_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            let Some(entry) = load_latest_turn_checkpoint_entry(
                session_id,
                config.memory.sliding_window,
                kernel_ctx,
                &memory_config,
            )
            .await?
            else {
                return Ok(TurnCheckpointTailRepairOutcome::no_checkpoint());
            };

            repair_turn_checkpoint_tail_entry(config, runtime, session_id, &entry, kernel_ctx).await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, kernel_ctx);
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
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
            let (summary, latest_entry) = load_turn_checkpoint_history_snapshot(
                session_id,
                limit,
                kernel_ctx,
                &memory_config,
            )
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
                                config, runtime, session_id, entry, kernel_ctx,
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
            let _ = (config, session_id, limit, runtime, kernel_ctx);
            Err(
                "turn checkpoint diagnostics unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }

    pub async fn probe_turn_checkpoint_tail_runtime_gate_with_runtime<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        self.probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit(
            config,
            session_id,
            config.memory.sliding_window,
            runtime,
            kernel_ctx,
        )
        .await
    }

    pub async fn probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        limit: usize,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        #[cfg(feature = "memory-sqlite")]
        {
            probe_turn_checkpoint_tail_runtime_gate_entry_with_limit(
                config, runtime, session_id, limit, kernel_ctx,
            )
            .await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, kernel_ctx);
            Err(
                "turn checkpoint runtime probe unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
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
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            &address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            kernel_ctx,
            None,
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
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
            None,
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
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            acp_options,
            kernel_ctx,
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
        kernel_ctx: Option<&KernelContext>,
        ingress: Option<&ConversationIngressContext>,
    ) -> CliResult<String> {
        let session_id = address.session_id.as_str();
        match evaluate_acp_conversation_turn_entry_for_address(config, address, acp_options)? {
            AcpConversationTurnEntryDecision::RejectExplicitWhenDisabled => {
                let error = "ACP is disabled by policy (`acp.enabled=false`)".to_owned();
                return match error_mode {
                    ProviderErrorMode::Propagate => Err(error),
                    ProviderErrorMode::InlineMessage => {
                        let synthetic = format_provider_error_reply(&error);
                        persist_reply_turns_raw_with_mode(
                            runtime,
                            session_id,
                            user_input,
                            &synthetic,
                            ReplyPersistenceMode::InlineProviderError,
                            kernel_ctx,
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

        if let Some(kernel_ctx) = kernel_ctx {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }
        let session_context = runtime.session_context(config, session_id, kernel_ctx)?;
        let tool_view = session_context.tool_view.clone();
        let visible_ingress = ingress.filter(|value| value.has_contextual_hints());
        emit_turn_ingress_event(runtime, session_id, visible_ingress, kernel_ctx).await;
        let preparation = ProviderTurnPreparation::from_assembled_context(
            config,
            runtime
                .build_context(config, session_id, true, kernel_ctx)
                .await?,
            user_input,
            visible_ingress,
        );
        let resolved_turn = resolve_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            runtime
                .request_turn(
                    config,
                    &preparation.session.messages,
                    &tool_view,
                    kernel_ctx,
                )
                .await,
            error_mode,
            kernel_ctx,
            ingress,
        )
        .await;

        apply_resolved_provider_turn(
            config,
            runtime,
            session_id,
            user_input,
            &preparation,
            &resolved_turn,
            kernel_ctx,
        )
        .await
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
        self.handle_turn_with_runtime_and_address_and_acp_options_and_ingress(
            config,
            address,
            user_input,
            error_mode,
            runtime,
            &acp_options,
            kernel_ctx,
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
                persist_reply_turns_raw_with_mode(
                    runtime,
                    session_id,
                    user_input,
                    &reply,
                    ReplyPersistenceMode::Success,
                    kernel_ctx,
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
                        persist_reply_turns_raw_with_mode(
                            runtime,
                            session_id,
                            user_input,
                            &synthetic,
                            ReplyPersistenceMode::InlineProviderError,
                            kernel_ctx,
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
) -> CliResult<ContextCompactionOutcome> {
    let estimated_tokens = estimated_tokens.or_else(|| estimate_tokens(messages));
    if !config
        .conversation
        .should_compact_with_estimate(messages.len(), estimated_tokens)
    {
        return Ok(ContextCompactionOutcome::Skipped);
    }
    let Some(kernel_ctx) = kernel_ctx else {
        return Ok(ContextCompactionOutcome::Skipped);
    };

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

async fn resolve_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    result: CliResult<ProviderTurn>,
    error_mode: ProviderErrorMode,
    kernel_ctx: Option<&KernelContext>,
    ingress: Option<&ConversationIngressContext>,
) -> ResolvedProviderTurn {
    match decide_provider_turn_request_action(result, error_mode) {
        ProviderTurnRequestAction::Continue { turn } => {
            let turn =
                scope_provider_turn_tool_intents(turn, session_id, preparation.turn_id.as_str());
            let continue_phase = prepare_provider_turn_continue_phase(
                config,
                runtime,
                session_id,
                preparation,
                turn,
                kernel_ctx,
                ingress,
            )
            .await;
            let reply = continue_phase
                .resolve_reply(runtime, preparation, user_input, kernel_ctx)
                .await;
            let checkpoint = continue_phase.checkpoint(preparation, user_input, reply.as_str());
            ResolvedProviderTurn::persist_reply(reply, checkpoint)
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
        if intent.session_id.trim().is_empty() {
            intent.session_id = session_id.to_owned();
        }
        if intent.turn_id.trim().is_empty() {
            intent.turn_id = turn_id.to_owned();
        }
    }
    turn
}

async fn prepare_provider_turn_continue_phase<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: ProviderTurn,
    kernel_ctx: Option<&KernelContext>,
    ingress: Option<&ConversationIngressContext>,
) -> ProviderTurnContinuePhase {
    let tool_intents = turn.tool_intents.len();
    let lane_execution = execute_provider_turn_lane(
        config,
        runtime,
        session_id,
        preparation,
        &turn,
        kernel_ctx,
        ingress,
    )
    .await;
    let followup_config =
        ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(config, &turn);
    ProviderTurnContinuePhase::new(tool_intents, lane_execution, followup_config)
}

async fn resolve_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    preparation: &ProviderTurnPreparation,
    lane_execution: &ProviderTurnLaneExecution,
    phase: &ToolDrivenReplyPhase,
    user_input: &str,
    kernel_ctx: Option<&KernelContext>,
) -> String {
    match phase.decision() {
        ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => reply.clone(),
        ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply,
            payload: followup,
        } => {
            let follow_up_messages = build_turn_reply_followup_messages(
                &preparation.session.messages,
                lane_execution.assistant_preface.as_str(),
                followup.clone(),
                user_input,
            );
            request_completion_with_raw_fallback(
                runtime,
                config,
                &follow_up_messages,
                kernel_ctx,
                raw_reply.as_str(),
            )
            .await
        }
    }
}

fn turn_checkpoint_result_kind(result: &TurnResult) -> TurnCheckpointResultKind {
    match result {
        TurnResult::FinalText(_) => TurnCheckpointResultKind::FinalText,
        TurnResult::NeedsApproval(_) => TurnCheckpointResultKind::NeedsApproval,
        TurnResult::ToolDenied(_) => TurnCheckpointResultKind::ToolDenied,
        TurnResult::ToolError(_) => TurnCheckpointResultKind::ToolError,
        TurnResult::ProviderError(_) => TurnCheckpointResultKind::ProviderError,
    }
}

fn format_turn_checkpoint_progress_status(status: TurnCheckpointProgressStatus) -> &'static str {
    match status {
        TurnCheckpointProgressStatus::Pending => "pending",
        TurnCheckpointProgressStatus::Skipped => "skipped",
        TurnCheckpointProgressStatus::Completed => "completed",
        TurnCheckpointProgressStatus::Failed => "failed",
        TurnCheckpointProgressStatus::FailedOpen => "failed_open",
    }
}

fn format_analytics_turn_checkpoint_progress_status(
    status: AnalyticsTurnCheckpointProgressStatus,
) -> &'static str {
    match status {
        AnalyticsTurnCheckpointProgressStatus::Pending => "pending",
        AnalyticsTurnCheckpointProgressStatus::Skipped => "skipped",
        AnalyticsTurnCheckpointProgressStatus::Completed => "completed",
        AnalyticsTurnCheckpointProgressStatus::Failed => "failed",
        AnalyticsTurnCheckpointProgressStatus::FailedOpen => "failed_open",
    }
}

fn build_turn_reply_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    messages.extend(build_tool_driven_followup_tail(
        assistant_preface,
        &followup,
        user_input,
        None,
        |_, text| text.to_owned(),
    ));
    messages
}

async fn persist_turn_checkpoint_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    let checkpoint = serde_json::to_value(checkpoint)
        .map_err(|error| format!("serialize turn checkpoint failed: {error}"))?;
    persist_turn_checkpoint_event_value(
        runtime,
        session_id,
        &checkpoint,
        stage,
        progress,
        failure,
        kernel_ctx,
    )
    .await
}

async fn persist_turn_checkpoint_event_value<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &Value,
    stage: TurnCheckpointStage,
    progress: TurnCheckpointFinalizationProgress,
    failure: Option<TurnCheckpointFailure>,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    persist_conversation_event(
        runtime,
        session_id,
        "turn_checkpoint",
        json!({
            "schema_version": 1,
            "stage": stage,
            "checkpoint": checkpoint,
            "finalization_progress": progress,
            "failure": failure,
        }),
        kernel_ctx,
    )
    .await
}

fn recover_latest_turn_pair(messages: &[Value]) -> Option<(String, String)> {
    let assistant_index = messages.iter().rposition(|message| {
        message.get("role").and_then(Value::as_str) == Some("assistant")
            && message.get("content").and_then(Value::as_str).is_some()
    })?;
    let assistant_reply = messages
        .get(assistant_index)?
        .get("content")
        .and_then(Value::as_str)?
        .to_owned();
    let user_input = messages
        .get(..assistant_index)?
        .iter()
        .rposition(|message| {
            message.get("role").and_then(Value::as_str) == Some("user")
                && message.get("content").and_then(Value::as_str).is_some()
        })
        .and_then(|index| {
            messages
                .get(index)
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)?;
    Some((user_input, assistant_reply))
}

fn load_turn_checkpoint_identity(checkpoint: &Value) -> Option<TurnCheckpointIdentity> {
    checkpoint
        .get("identity")
        .cloned()
        .and_then(|identity| serde_json::from_value(identity).ok())
}

fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

fn checkpoint_context_fingerprint_sha256(messages: &[Value]) -> String {
    let serialized = Value::Array(messages.to_vec()).to_string();
    format!("{:x}", Sha256::digest(serialized.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
struct TurnCheckpointRepairPreparation {
    #[serde(default)]
    context_message_count: Option<usize>,
    #[serde(default)]
    context_fingerprint_sha256: Option<String>,
    #[serde(default)]
    estimated_tokens: Option<usize>,
}

fn load_turn_checkpoint_repair_preparation(
    checkpoint: &Value,
) -> Result<Option<TurnCheckpointRepairPreparation>, TurnCheckpointTailRepairReason> {
    let Some(preparation) = checkpoint.get("preparation") else {
        return Ok(None);
    };
    serde_json::from_value(preparation.clone())
        .map(Some)
        .map_err(|_error| TurnCheckpointTailRepairReason::CheckpointPreparationMalformed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnCheckpointRepairResumeInput {
    user_input: String,
    assistant_reply: String,
    messages: Vec<Value>,
    estimated_tokens: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TurnCheckpointTailRuntimeEligibility {
    NotNeeded {
        action: TurnCheckpointRecoveryAction,
        reason: TurnCheckpointTailRepairReason,
    },
    Manual {
        action: TurnCheckpointRecoveryAction,
        reason: TurnCheckpointTailRepairReason,
        source: TurnCheckpointTailRepairSource,
    },
    Runnable {
        action: TurnCheckpointRecoveryAction,
        plan: TurnCheckpointRepairPlan,
        resume_input: TurnCheckpointRepairResumeInput,
    },
}

impl TurnCheckpointRepairResumeInput {
    fn from_assembled_context(
        assembled: AssembledConversationContext,
        checkpoint: &Value,
    ) -> Result<Self, TurnCheckpointTailRepairReason> {
        let repair_preparation = load_turn_checkpoint_repair_preparation(checkpoint)?;
        let messages = assembled.messages;
        let Some((user_input, assistant_reply)) = recover_latest_turn_pair(&messages) else {
            return Err(TurnCheckpointTailRepairReason::VisibleTurnPairMissing);
        };
        let Some((_, pre_assistant_messages)) = messages.split_last() else {
            return Err(TurnCheckpointTailRepairReason::VisibleTurnPairMissing);
        };
        let Some(identity) = load_turn_checkpoint_identity(checkpoint) else {
            return Err(TurnCheckpointTailRepairReason::CheckpointIdentityMissing);
        };
        if !identity.matches_turn(&user_input, &assistant_reply) {
            return Err(TurnCheckpointTailRepairReason::CheckpointIdentityMismatch);
        }
        if let Some(expected_context_message_count) = repair_preparation
            .as_ref()
            .and_then(|preparation| preparation.context_message_count)
            && pre_assistant_messages.len() != expected_context_message_count
        {
            return Err(TurnCheckpointTailRepairReason::CheckpointPreparationMismatch);
        }
        if let Some(expected_context_fingerprint_sha256) = repair_preparation
            .as_ref()
            .and_then(|preparation| preparation.context_fingerprint_sha256.as_deref())
            && checkpoint_context_fingerprint_sha256(pre_assistant_messages)
                != expected_context_fingerprint_sha256
        {
            return Err(TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch);
        }

        Ok(Self {
            user_input,
            assistant_reply,
            messages,
            estimated_tokens: repair_preparation
                .and_then(|preparation| preparation.estimated_tokens)
                .or(assembled.estimated_tokens),
        })
    }
}

#[cfg(feature = "memory-sqlite")]
async fn repair_turn_checkpoint_tail_entry<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    entry: &super::session_history::TurnCheckpointLatestEntry,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<TurnCheckpointTailRepairOutcome> {
    let summary = &entry.summary;
    let (action, repair_plan, resume_input) = match load_turn_checkpoint_tail_runtime_eligibility(
        config, runtime, session_id, entry, kernel_ctx,
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
        let Some(kernel_ctx) = kernel_ctx else {
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
                kernel_ctx,
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
                &resume_input.user_input,
                &resume_input.assistant_reply,
                &resume_input.messages,
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
                    Some(kernel_ctx),
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
            &resume_input.messages,
            resume_input.estimated_tokens,
            kernel_ctx,
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
                    kernel_ctx,
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
        kernel_ctx,
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
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    match load_turn_checkpoint_tail_runtime_eligibility(
        config, runtime, session_id, entry, kernel_ctx,
    )
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
    kernel_ctx: Option<&KernelContext>,
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
    let assembled = runtime
        .build_context(config, session_id, true, kernel_ctx)
        .await?;
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
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let Some(entry) =
        load_latest_turn_checkpoint_entry(session_id, limit, kernel_ctx, &memory_config).await?
    else {
        return Ok(None);
    };
    probe_turn_checkpoint_tail_runtime_gate_entry(config, runtime, session_id, &entry, kernel_ctx)
        .await
}

fn restore_analytics_turn_checkpoint_progress_status(
    status: AnalyticsTurnCheckpointProgressStatus,
) -> TurnCheckpointProgressStatus {
    match status {
        AnalyticsTurnCheckpointProgressStatus::Pending => TurnCheckpointProgressStatus::Pending,
        AnalyticsTurnCheckpointProgressStatus::Skipped => TurnCheckpointProgressStatus::Skipped,
        AnalyticsTurnCheckpointProgressStatus::Completed => TurnCheckpointProgressStatus::Completed,
        AnalyticsTurnCheckpointProgressStatus::Failed => TurnCheckpointProgressStatus::Failed,
        AnalyticsTurnCheckpointProgressStatus::FailedOpen => {
            TurnCheckpointProgressStatus::FailedOpen
        }
    }
}

async fn finalize_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    tail_phase: &ProviderTurnReplyTailPhase,
    checkpoint: &TurnCheckpointSnapshot,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    let Some(persistence_mode) = checkpoint.finalization.persistence_mode() else {
        return Ok(tail_phase.reply().to_owned());
    };
    persist_reply_turns_with_mode(
        runtime,
        session_id,
        user_input,
        tail_phase.reply(),
        persistence_mode,
        kernel_ctx,
    )
    .await?;

    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::PostPersist,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        kernel_ctx,
    )
    .await?;

    let after_turn_status = if checkpoint.finalization.runs_after_turn() {
        if let Some(kernel_ctx) = kernel_ctx {
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
                        Some(kernel_ctx),
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
            kernel_ctx,
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
                    kernel_ctx,
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
        kernel_ctx,
    )
    .await?;
    Ok(tail_phase.reply().to_owned())
}

async fn persist_resolved_provider_error_checkpoint<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        kernel_ctx,
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
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<String> {
    resolved
        .terminal_phase(&preparation.session)
        .apply(config, runtime, session_id, user_input, kernel_ctx)
        .await
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

#[cfg(feature = "memory-sqlite")]
struct CoordinatorApprovalResolutionRuntime<'a, R: ?Sized> {
    config: &'a LoongClawConfig,
    runtime: &'a R,
    fallback: &'a DefaultAppToolDispatcher,
    kernel_ctx: Option<&'a KernelContext>,
}

#[cfg(feature = "memory-sqlite")]
impl<R> CoordinatorApprovalResolutionRuntime<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    fn current_epoch_s() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    }

    fn replay_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loongclaw_contracts::ToolCoreRequest, String> {
        let execution_kind = approval_request
            .request_payload_json
            .get("execution_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval_request_invalid_payload: missing execution_kind".to_owned())?;
        if execution_kind != "app" {
            return Err(format!(
                "approval_request_invalid_execution_kind: expected `app`, got `{execution_kind}`"
            ));
        }

        let tool_name = approval_request
            .request_payload_json
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "approval_request_invalid_payload: missing tool_name".to_owned())?;
        let payload = approval_request
            .request_payload_json
            .get("args_json")
            .cloned()
            .ok_or_else(|| "approval_request_invalid_payload: missing args_json".to_owned())?;

        Ok(loongclaw_contracts::ToolCoreRequest {
            tool_name: tool_name.to_owned(),
            payload,
        })
    }

    async fn replay_approved_request(
        &self,
        approval_request: &ApprovalRequestRecord,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        let replay_request = self.replay_request(approval_request)?;
        let session_context = self
            .runtime
            .session_context(self.config, &approval_request.session_id, self.kernel_ctx)
            .map_err(|error| format!("load approval request session context failed: {error}"))?;

        match crate::tools::canonical_tool_name(replay_request.tool_name.as_str()) {
            "delegate" => {
                execute_delegate_tool(
                    self.config,
                    self.runtime,
                    &session_context,
                    replay_request.payload,
                    self.kernel_ctx,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    &session_context,
                    replay_request.payload,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(&session_context, replay_request, self.kernel_ctx)
                    .await
            }
        }
    }

    async fn execute_approved_request(
        &self,
        repo: &SessionRepository,
        approval_request_id: &str,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let executing = repo
            .transition_approval_request_if_current(
                approval_request_id,
                TransitionApprovalRequestIfCurrentRequest {
                    expected_status: ApprovalRequestStatus::Approved,
                    next_status: ApprovalRequestStatus::Executing,
                    decision: None,
                    resolved_by_session_id: None,
                    executed_at: None,
                    last_error: None,
                },
            )?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_approved: `{approval_request_id}` is no longer approved"
                )
            })?;

        match self.replay_approved_request(&executing).await {
            Ok(resumed_tool_output) => {
                let executed = repo
                    .transition_approval_request_if_current(
                        approval_request_id,
                        TransitionApprovalRequestIfCurrentRequest {
                            expected_status: ApprovalRequestStatus::Executing,
                            next_status: ApprovalRequestStatus::Executed,
                            decision: None,
                            resolved_by_session_id: None,
                            executed_at: Some(Self::current_epoch_s()),
                            last_error: None,
                        },
                    )?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_not_executing: `{approval_request_id}` is no longer executing"
                        )
                    })?;
                Ok(crate::tools::approval::ApprovalResolutionOutcome {
                    approval_request: executed,
                    resumed_tool_output: Some(resumed_tool_output),
                })
            }
            Err(error) => {
                let _ = repo.transition_approval_request_if_current(
                    approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Executing,
                        next_status: ApprovalRequestStatus::Executed,
                        decision: None,
                        resolved_by_session_id: None,
                        executed_at: Some(Self::current_epoch_s()),
                        last_error: Some(error.clone()),
                    },
                )?;
                Err(error)
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> crate::tools::approval::ApprovalResolutionRuntime
    for CoordinatorApprovalResolutionRuntime<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    async fn resolve_approval_request(
        &self,
        request: crate::tools::approval::ApprovalResolutionRequest,
    ) -> Result<crate::tools::approval::ApprovalResolutionOutcome, String> {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&self.config.memory);
        let repo = SessionRepository::new(&memory_config)?;
        let approval_request = repo
            .load_approval_request(&request.approval_request_id)?
            .ok_or_else(|| {
                format!(
                    "approval_request_not_found: `{}`",
                    request.approval_request_id
                )
            })?;

        let is_visible = match request.visibility {
            crate::config::SessionVisibility::SelfOnly => {
                request.current_session_id == approval_request.session_id
            }
            crate::config::SessionVisibility::Children => {
                request.current_session_id == approval_request.session_id
                    || repo.is_session_visible(
                        &request.current_session_id,
                        &approval_request.session_id,
                    )?
            }
        };
        if !is_visible {
            return Err(format!(
                "visibility_denied: session `{}` is not visible from `{}`",
                approval_request.session_id, request.current_session_id
            ));
        }

        match request.decision {
            ApprovalDecision::Deny => {
                let resolved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Denied,
                        decision: Some(ApprovalDecision::Deny),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(resolved) => resolved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                Ok(crate::tools::approval::ApprovalResolutionOutcome {
                    approval_request: resolved,
                    resumed_tool_output: None,
                })
            }
            ApprovalDecision::ApproveOnce => {
                let approved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Approved,
                        decision: Some(ApprovalDecision::ApproveOnce),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(approved) => approved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                let _ = approved;
                self.execute_approved_request(&repo, &request.approval_request_id)
                    .await
            }
            ApprovalDecision::ApproveAlways => {
                let approved = match repo.transition_approval_request_if_current(
                    &request.approval_request_id,
                    TransitionApprovalRequestIfCurrentRequest {
                        expected_status: ApprovalRequestStatus::Pending,
                        next_status: ApprovalRequestStatus::Approved,
                        decision: Some(ApprovalDecision::ApproveAlways),
                        resolved_by_session_id: Some(request.current_session_id.clone()),
                        executed_at: None,
                        last_error: None,
                    },
                )? {
                    Some(approved) => approved,
                    None => {
                        let latest = repo
                            .load_approval_request(&request.approval_request_id)?
                            .ok_or_else(|| {
                                format!(
                                    "approval_request_not_found: `{}`",
                                    request.approval_request_id
                                )
                            })?;
                        return Err(format!(
                            "approval_request_not_pending: `{}` is already {}",
                            request.approval_request_id,
                            latest.status.as_str()
                        ));
                    }
                };
                let grant_scope_session_id = repo
                    .lineage_root_session_id(&approved.session_id)?
                    .ok_or_else(|| {
                        format!(
                            "approval_request_session_not_found: `{}`",
                            approved.session_id
                        )
                    })?;
                repo.upsert_approval_grant(NewApprovalGrantRecord {
                    scope_session_id: grant_scope_session_id,
                    approval_key: approved.approval_key.clone(),
                    created_by_session_id: Some(request.current_session_id.clone()),
                })?;
                self.execute_approved_request(&repo, &request.approval_request_id)
                    .await
            }
        }
    }
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
    async fn maybe_require_approval(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<Option<super::turn_engine::ApprovalRequirement>, String> {
        self.fallback
            .maybe_require_approval(session_context, intent, descriptor, kernel_ctx)
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loongclaw_contracts::ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
            "approval_request_resolve" => {
                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, kernel_ctx);
                    Err("approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                        .to_owned())
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let memory_config =
                        MemoryRuntimeConfig::from_memory_config(&self.config.memory);
                    let effective_tool_config =
                        effective_tool_config_for_session(&self.config.tools, session_context);
                    let approval_runtime = CoordinatorApprovalResolutionRuntime {
                        config: self.config,
                        runtime: self.runtime,
                        fallback: self.fallback,
                        kernel_ctx,
                    };
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
                    kernel_ctx,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
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
async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
    kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let delegate_request = crate::tools::delegate::parse_delegate_request_with_default_timeout(
        &payload,
        config.tools.delegate.timeout_seconds,
    )?;
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_request.label.clone();
    let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(&config.memory))?;
    let current_depth = repo.session_lineage_depth(&session_context.session_id)?;
    let next_child_depth = current_depth.saturating_add(1);
    if next_child_depth > config.tools.delegate.max_depth {
        return Err(format!(
            "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {}",
            config.tools.delegate.max_depth
        ));
    }

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
            "task": delegate_request.task.clone(),
            "label": child_label.clone(),
            "timeout_seconds": delegate_request.timeout_seconds,
        }),
    })?;

    run_started_delegate_child_turn_with_runtime(
        config,
        runtime,
        &child_session_id,
        &session_context.session_id,
        child_label,
        &delegate_request.task,
        delegate_request.timeout_seconds,
        kernel_ctx,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let runtime_handle = Handle::try_current()
        .map_err(|error| format!("delegate_async_runtime_unavailable: {error}"))?;
    let spawner = runtime
        .async_delegate_spawner(config)
        .ok_or_else(|| "delegate_async_not_configured".to_owned())?;
    let delegate_request = crate::tools::delegate::parse_delegate_request_with_default_timeout(
        &payload,
        config.tools.delegate.timeout_seconds,
    )?;
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_request.label.clone();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;
    let current_depth = repo.session_lineage_depth(&session_context.session_id)?;
    let next_child_depth = current_depth.saturating_add(1);
    if next_child_depth > config.tools.delegate.max_depth {
        return Err(format!(
            "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {}",
            config.tools.delegate.max_depth
        ));
    }

    repo.create_session_with_event(CreateSessionWithEventRequest {
        session: NewSessionRecord {
            session_id: child_session_id.clone(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(session_context.session_id.clone()),
            label: child_label.clone(),
            state: SessionState::Ready,
        },
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some(session_context.session_id.clone()),
        event_payload_json: json!({
            "task": delegate_request.task,
            "label": child_label,
            "timeout_seconds": delegate_request.timeout_seconds,
        }),
    })?;

    spawn_async_delegate_detached(
        runtime_handle,
        memory_config,
        spawner,
        AsyncDelegateSpawnRequest {
            child_session_id: child_session_id.clone(),
            parent_session_id: session_context.session_id.clone(),
            task: delegate_request.task,
            label: child_label,
            timeout_seconds: delegate_request.timeout_seconds,
        },
    );

    Ok(crate::tools::delegate::delegate_async_queued_outcome(
        child_session_id,
        delegate_request.label,
        delegate_request.timeout_seconds,
    ))
}

#[cfg(not(feature = "memory-sqlite"))]
async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongClawConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
    _kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    Err("delegate requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(not(feature = "memory-sqlite"))]
async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongClawConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
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
    timeout_seconds: u64,
    kernel_ctx: Option<&KernelContext>,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(&config.memory))?;
    let start = Instant::now();
    let child_result = timeout(Duration::from_secs(timeout_seconds), async {
        AssertUnwindSafe(ConversationTurnCoordinator::new().handle_turn_with_runtime(
            config,
            child_session_id,
            user_input,
            ProviderErrorMode::Propagate,
            runtime,
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
                child_label,
                final_output,
                turn_count,
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
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
                child_label,
                error.clone(),
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(error.clone()),
                    event_kind: "delegate_failed".to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
                    event_payload_json: json!({
                        "error": error,
                        "duration_ms": duration_ms,
                    }),
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
                child_label,
                panic_error.clone(),
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(panic_error.clone()),
                    event_kind: "delegate_failed".to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
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
                child_label,
                duration_ms,
            );
            finalize_delegate_child_terminal_with_recovery(
                &repo,
                child_session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::TimedOut,
                    last_error: Some(timeout_error.clone()),
                    event_kind: "delegate_timed_out".to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
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
fn finalize_async_delegate_spawn_failure(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let repo = SessionRepository::new(memory_config)?;
    let outcome = crate::tools::delegate::delegate_error_outcome(
        child_session_id.to_owned(),
        label,
        error.clone(),
        0,
    );
    let request = FinalizeSessionTerminalRequest {
        state: SessionState::Failed,
        last_error: Some(error.clone()),
        event_kind: "delegate_spawn_failed".to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json: json!({
            "error": error,
        }),
        outcome_status: outcome.status,
        outcome_payload_json: outcome.payload,
    };
    finalize_terminal_if_current_allowing_stale_state(
        &repo,
        child_session_id,
        SessionState::Ready,
        request,
    )?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let recovery_label = label.clone();
    match finalize_async_delegate_spawn_failure(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        error.clone(),
    ) {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let repo = SessionRepository::new(memory_config)?;
            let recovery_error = format!(
                "delegate_async_spawn_failure_persist_failed: {finalize_error}; original spawn error: {error}"
            );
            match repo.transition_session_with_event_if_current(
                child_session_id,
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Ready,
                    next_state: SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
                    event_payload_json: build_async_spawn_failure_recovery_payload(
                        recovery_label.as_deref(),
                        &error,
                        &recovery_error,
                    ),
                },
            ) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => {
                    let current_state = repo
                        .load_session(child_session_id)?
                        .map(|session| session.state.as_str().to_owned())
                        .unwrap_or_else(|| "missing".to_owned());
                    Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                    ))
                }
                Err(recovery_event_error) => match repo.update_session_state_if_current(
                    child_session_id,
                    SessionState::Ready,
                    SessionState::Failed,
                    Some(recovery_error.clone()),
                ) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => {
                        let current_state = repo
                            .load_session(child_session_id)?
                            .map(|session| session.state.as_str().to_owned())
                            .unwrap_or_else(|| "missing".to_owned());
                        Err(format!(
                            "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                        ))
                    }
                    Err(mark_error) => Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_failed: {mark_error}; delegate_async_spawn_recovery_event_failed: {recovery_event_error}"
                    )),
                },
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn format_async_delegate_spawn_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_async_spawn_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };
    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_async_spawn_panic: {}", *message),
        Err(_) => "delegate_async_spawn_panic".to_owned(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn spawn_async_delegate_detached(
    runtime_handle: Handle,
    memory_config: MemoryRuntimeConfig,
    spawner: std::sync::Arc<dyn AsyncDelegateSpawner>,
    request: AsyncDelegateSpawnRequest,
) {
    let child_session_id = request.child_session_id.clone();
    let parent_session_id = request.parent_session_id.clone();
    let label = request.label.clone();
    runtime_handle.spawn(async move {
        let spawn_failure = match AssertUnwindSafe(spawner.spawn(request))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(panic_payload) => Some(format_async_delegate_spawn_panic(panic_payload)),
        };
        if let Some(error) = spawn_failure {
            let _ = finalize_async_delegate_spawn_failure_with_recovery(
                &memory_config,
                &child_session_id,
                &parent_session_id,
                label,
                error,
            );
        }
    });
}

#[cfg(feature = "memory-sqlite")]
fn finalize_delegate_child_terminal_with_recovery(
    repo: &SessionRepository,
    child_session_id: &str,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    let recovery_request = request.clone();
    match finalize_terminal_if_current_allowing_stale_state(
        repo,
        child_session_id,
        SessionState::Running,
        request,
    ) {
        Ok(()) => Ok(()),
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
fn finalize_terminal_if_current_allowing_stale_state(
    repo: &SessionRepository,
    session_id: &str,
    expected_state: SessionState,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    match repo.finalize_session_terminal_if_current(session_id, expected_state, request)? {
        Some(_) => Ok(()),
        None => {
            if repo.load_session(session_id)?.is_some() {
                Ok(())
            } else {
                Err(format!("session `{session_id}` not found"))
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

async fn execute_provider_turn_lane<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: &ProviderTurn,
    kernel_ctx: Option<&KernelContext>,
    ingress: Option<&ConversationIngressContext>,
) -> ProviderTurnLaneExecution {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let assistant_preface = turn.assistant_text.clone();
    let lane = preparation.lane_plan.decision.lane;
    let session_context = match runtime.session_context(config, session_id, kernel_ctx) {
        Ok(session_context) => session_context,
        Err(error) => {
            return ProviderTurnLaneExecution {
                lane,
                assistant_preface,
                had_tool_intents,
                raw_tool_output_requested: preparation.raw_tool_output_requested,
                turn_result: TurnResult::non_retryable_tool_error("session_context_failed", error),
                safe_lane_terminal_route: None,
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
    let use_safe_lane_plan_path = preparation
        .lane_plan
        .should_use_safe_lane_plan_path(config, turn);
    let engine = TurnEngine::with_tool_result_payload_summary_limit(
        preparation.lane_plan.max_tool_steps,
        payload_summary_limit_chars,
    );
    let validation = if use_safe_lane_plan_path {
        TurnEngine::with_tool_result_payload_summary_limit(usize::MAX, payload_summary_limit_chars)
            .validate_turn_in_context(turn, &session_context)
    } else {
        engine.validate_turn_in_context(turn, &session_context)
    };
    let (turn_result, safe_lane_terminal_route) = match validation {
        Ok(TurnValidation::FinalText(text)) => (TurnResult::FinalText(text), None),
        Err(failure) => (TurnResult::ToolDenied(failure), None),
        Ok(TurnValidation::ToolExecutionRequired) if use_safe_lane_plan_path => {
            let outcome = execute_turn_with_safe_lane_plan(
                config,
                runtime,
                session_id,
                &preparation.lane_plan.decision,
                turn,
                &session_context,
                &app_dispatcher,
                kernel_ctx,
                ingress,
            )
            .await;
            (outcome.result, outcome.terminal_route)
        }
        Ok(TurnValidation::ToolExecutionRequired) => (
            engine
                .execute_turn_in_context(
                    turn,
                    &session_context,
                    &app_dispatcher,
                    kernel_ctx,
                    ingress,
                )
                .await,
            None,
        ),
    };

    ProviderTurnLaneExecution {
        lane,
        assistant_preface,
        had_tool_intents,
        raw_tool_output_requested: preparation.raw_tool_output_requested,
        turn_result,
        safe_lane_terminal_route,
    }
}

async fn execute_turn_with_safe_lane_plan<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    session_id: &str,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    kernel_ctx: Option<&KernelContext>,
    ingress: Option<&ConversationIngressContext>,
) -> SafeLaneTurnOutcome {
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
                kernel_ctx,
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
            kernel_ctx,
        )
        .await;

        let round_execution = evaluate_safe_lane_round(
            config,
            lane_decision,
            turn,
            session_context,
            app_dispatcher,
            kernel_ctx,
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
                    kernel_ctx,
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
                        kernel_ctx,
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
                    state.governor,
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
                    kernel_ctx,
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
                            kernel_ctx,
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
                            kernel_ctx,
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
                    state.governor,
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
                    kernel_ctx,
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
                            kernel_ctx,
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
                            kernel_ctx,
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
    kernel_ctx: Option<&KernelContext>,
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
        kernel_ctx,
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

async fn emit_turn_ingress_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    ingress: Option<&ConversationIngressContext>,
    kernel_ctx: Option<&KernelContext>,
) {
    let Some(ingress) = ingress else {
        return;
    };
    let _ = persist_conversation_event(
        runtime,
        session_id,
        "turn_ingress",
        ingress.as_event_payload(),
        kernel_ctx,
    )
    .await;
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
    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        if let Ok(assistant_contents) = load_assistant_contents_from_session_window(
            session_id,
            window_turns,
            kernel_ctx,
            &memory_config,
        )
        .await
        {
            return summarize_governor_history_signals(
                assistant_contents.iter().map(String::as_str),
            );
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
    let projection = summarize_safe_lane_history(assistant_contents);
    SafeLaneGovernorHistorySignals {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct SafeLaneFailureRoute {
    decision: SafeLaneFailureRouteDecision,
    reason: SafeLaneFailureRouteReason,
    source: SafeLaneFailureRouteSource,
}

impl SafeLaneFailureRoute {
    fn from_failure(failure: &TurnFailure, replan_budget: SafeLaneReplanBudget) -> Self {
        if let SafeLaneContinuationBudgetDecision::Terminal { reason } =
            replan_budget.continuation_decision()
        {
            return Self::terminal(reason);
        }

        match failure.code.as_str() {
            "kernel_policy_denied"
            | "tool_not_found"
            | "max_tool_steps_exceeded"
            | "no_kernel_context" => {
                return Self::terminal(SafeLaneFailureRouteReason::PolicyDenied);
            }
            "tool_execution_failed" => {
                if failure.retryable {
                    return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                }
                return Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse);
            }
            "kernel_execution_failed" => {
                return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
            }
            _ => {}
        }

        if let Some(code) = SafeLaneFailureCode::parse(failure.code.as_str()) {
            match code {
                SafeLaneFailureCode::PlanNodePolicyDenied => {
                    return Self::terminal(SafeLaneFailureRouteReason::PolicyDenied);
                }
                SafeLaneFailureCode::VerifyFailed => {
                    if failure.retryable {
                        return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                    }
                    return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
                }
                SafeLaneFailureCode::PlanNodeRetryableError => {
                    if failure.retryable {
                        return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                    }
                    return Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse);
                }
                SafeLaneFailureCode::VerifyFailedBudgetExhausted => {
                    return Self::terminal(SafeLaneFailureRouteReason::RoundBudgetExhausted);
                }
                SafeLaneFailureCode::PlanValidationFailed
                | SafeLaneFailureCode::PlanTopologyResolutionFailed
                | SafeLaneFailureCode::PlanBudgetExceeded
                | SafeLaneFailureCode::PlanWallTimeExceeded
                | SafeLaneFailureCode::PlanNodeNonRetryableError
                | SafeLaneFailureCode::VerifyFailedBackpressureGuard
                | SafeLaneFailureCode::VerifyFailedSessionGovernor
                | SafeLaneFailureCode::PlanBackpressureGuard
                | SafeLaneFailureCode::PlanSessionGovernorNoReplan => {
                    return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
                }
            }
        }

        match failure.kind {
            TurnFailureKind::Retryable if failure.retryable => {
                Self::replan(SafeLaneFailureRouteReason::RetryableFailure)
            }
            TurnFailureKind::Retryable => {
                Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse)
            }
            TurnFailureKind::PolicyDenied => {
                Self::terminal(SafeLaneFailureRouteReason::PolicyDenied)
            }
            TurnFailureKind::NonRetryable => {
                Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure)
            }
            TurnFailureKind::Provider => {
                Self::terminal(SafeLaneFailureRouteReason::ProviderFailure)
            }
        }
    }

    fn replan(reason: SafeLaneFailureRouteReason) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason,
            source: SafeLaneFailureRouteSource::BaseRouting,
        }
    }

    fn terminal(reason: SafeLaneFailureRouteReason) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason,
            source: SafeLaneFailureRouteSource::BaseRouting,
        }
    }

    fn terminal_with_source(
        reason: SafeLaneFailureRouteReason,
        source: SafeLaneFailureRouteSource,
    ) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason,
            source,
        }
    }

    fn is_base_round_budget_terminal(self) -> bool {
        self.decision == SafeLaneFailureRouteDecision::Terminal
            && self.source == SafeLaneFailureRouteSource::BaseRouting
            && self.reason == SafeLaneFailureRouteReason::RoundBudgetExhausted
    }

    fn should_replan(self) -> bool {
        self.decision == SafeLaneFailureRouteDecision::Replan
    }

    fn decision_label(self) -> &'static str {
        self.decision.as_str()
    }

    fn source_label(self) -> &'static str {
        self.source.as_str()
    }

    fn verify_terminal_summary_label(self) -> &'static str {
        match (self.source, self.reason) {
            (SafeLaneFailureRouteSource::BackpressureGuard, _) => {
                "verify_failed_backpressure_guard"
            }
            (SafeLaneFailureRouteSource::SessionGovernor, _) => "verify_failed_session_governor",
            (
                SafeLaneFailureRouteSource::BaseRouting,
                SafeLaneFailureRouteReason::RoundBudgetExhausted,
            ) => "verify_failed_budget_exhausted",
            (SafeLaneFailureRouteSource::BaseRouting, _) => "verify_failed_non_retryable",
        }
    }

    fn terminal_verify_failure_code(self, retryable_signal: bool) -> SafeLaneFailureCode {
        match (self.source, self.reason, retryable_signal) {
            (SafeLaneFailureRouteSource::BackpressureGuard, _, _) => {
                SafeLaneFailureCode::VerifyFailedBackpressureGuard
            }
            (SafeLaneFailureRouteSource::SessionGovernor, _, _) => {
                SafeLaneFailureCode::VerifyFailedSessionGovernor
            }
            (
                SafeLaneFailureRouteSource::BaseRouting,
                SafeLaneFailureRouteReason::RoundBudgetExhausted,
                true,
            ) => SafeLaneFailureCode::VerifyFailedBudgetExhausted,
            (SafeLaneFailureRouteSource::BaseRouting, _, _) => SafeLaneFailureCode::VerifyFailed,
        }
    }

    fn terminal_plan_failure_code(self) -> Option<SafeLaneFailureCode> {
        match self.source {
            SafeLaneFailureRouteSource::BackpressureGuard => {
                Some(SafeLaneFailureCode::PlanBackpressureGuard)
            }
            SafeLaneFailureRouteSource::SessionGovernor => {
                Some(SafeLaneFailureCode::PlanSessionGovernorNoReplan)
            }
            SafeLaneFailureRouteSource::BaseRouting => None,
        }
    }

    fn with_backpressure_guard(
        self,
        backpressure_budget: Option<SafeLaneBackpressureBudget>,
        metrics: SafeLaneExecutionMetrics,
    ) -> Self {
        if !self.should_replan() {
            return self;
        }

        let Some(reason) = backpressure_budget.and_then(|budget| {
            match budget
                .continuation_decision(metrics.total_attempts_used, metrics.replans_triggered)
            {
                SafeLaneContinuationBudgetDecision::Continue => None,
                SafeLaneContinuationBudgetDecision::Terminal { reason } => Some(reason),
            }
        }) else {
            return self;
        };

        Self::terminal_with_source(reason, SafeLaneFailureRouteSource::BackpressureGuard)
    }

    fn with_session_governor_override(self, governor: SafeLaneSessionGovernorDecision) -> Self {
        if governor.force_no_replan && self.is_base_round_budget_terminal() {
            return Self::terminal_with_source(
                SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                SafeLaneFailureRouteSource::SessionGovernor,
            );
        }
        self
    }
}

#[derive(Debug, Clone)]
enum SafeLaneRoundDecision {
    Finalize {
        result: TurnResult,
    },
    Replan {
        reason: String,
        next_plan_start_tool_index: usize,
        next_seed_tool_outputs: Vec<String>,
    },
}

fn decide_safe_lane_failure_route(
    config: &LoongClawConfig,
    failure: &TurnFailure,
    replan_budget: SafeLaneReplanBudget,
    metrics: SafeLaneExecutionMetrics,
    governor: SafeLaneSessionGovernorDecision,
) -> SafeLaneFailureRoute {
    SafeLaneFailureRoute::from_failure(failure, replan_budget)
        .with_backpressure_guard(safe_lane_backpressure_budget(config), metrics)
        .with_session_governor_override(governor)
}

fn decide_safe_lane_verify_failure_action(
    verify_error: &str,
    retryable_signal: bool,
    route: SafeLaneFailureRoute,
) -> SafeLaneRoundDecision {
    if !route.should_replan() {
        return SafeLaneRoundDecision::Finalize {
            result: TurnResult::ToolError(terminal_turn_failure_from_verify_failure(
                verify_error,
                retryable_signal,
                route,
            )),
        };
    }

    SafeLaneRoundDecision::Replan {
        reason: "verify_failed".to_owned(),
        next_plan_start_tool_index: 0,
        next_seed_tool_outputs: Vec::new(),
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

fn derive_replan_cursor(
    failure: &PlanRunFailure,
    round_tool_outputs: &[String],
    tool_count: usize,
) -> (usize, Vec<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match failure {
        PlanRunFailure::NodeFailed { node_id, .. } => {
            if let Ok(index) = parse_tool_node_index(node_id.as_str())
                && index < tool_count
            {
                return (index, round_tool_outputs.to_vec());
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
        TurnFailureKind::PolicyDenied => "policy_denied",
        TurnFailureKind::Retryable => "retryable",
        TurnFailureKind::NonRetryable => "non_retryable",
        TurnFailureKind::Provider => "provider",
    }
}

fn turn_failure_from_plan_failure(failure: &PlanRunFailure) -> TurnFailure {
    let (code, kind) = classify_safe_lane_plan_failure(failure);
    match failure {
        PlanRunFailure::ValidationFailed(error) => {
            code.into_turn_failure(kind, format!("{}: {error}", code.as_str()))
        }
        PlanRunFailure::TopologyResolutionFailed => code.into_turn_failure(kind, code.as_str()),
        PlanRunFailure::BudgetExceeded {
            attempts_used,
            limit,
        } => code.into_turn_failure(
            kind,
            format!(
                "{} attempts_used={attempts_used} limit={limit}",
                code.as_str()
            ),
        ),
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms,
            limit_ms,
        } => code.into_turn_failure(
            kind,
            format!(
                "{} elapsed_ms={elapsed_ms} limit_ms={limit_ms}",
                code.as_str()
            ),
        ),
        PlanRunFailure::NodeFailed { last_error, .. } => {
            code.into_turn_failure(kind, last_error.clone())
        }
    }
}

fn turn_failure_from_verify_failure(verify_error: &str, retryable: bool) -> TurnFailure {
    SafeLaneFailureCode::VerifyFailed.into_turn_failure(
        if retryable {
            TurnFailureKind::Retryable
        } else {
            TurnFailureKind::NonRetryable
        },
        format!(
            "{}: {verify_error}",
            SafeLaneFailureCode::VerifyFailed.as_str()
        ),
    )
}

fn terminal_turn_failure_from_verify_failure(
    verify_error: &str,
    retryable_signal: bool,
    route: SafeLaneFailureRoute,
) -> TurnFailure {
    route
        .terminal_verify_failure_code(retryable_signal)
        .into_turn_failure(
            TurnFailureKind::NonRetryable,
            format!(
                "{}: {verify_error}",
                SafeLaneFailureCode::VerifyFailed.as_str()
            ),
        )
}

fn turn_result_from_plan_failure(failure: PlanRunFailure) -> TurnResult {
    let failure_meta = turn_failure_from_plan_failure(&failure);
    if matches!(failure_meta.kind, TurnFailureKind::PolicyDenied) {
        TurnResult::ToolDenied(failure_meta)
    } else if matches!(failure_meta.kind, TurnFailureKind::Provider) {
        TurnResult::ProviderError(failure_meta)
    } else {
        TurnResult::ToolError(failure_meta)
    }
}

fn terminal_turn_result_from_plan_failure_with_route(
    failure: PlanRunFailure,
    route: SafeLaneFailureRoute,
) -> TurnResult {
    if let Some(code) = route.terminal_plan_failure_code() {
        let summary = summarize_plan_failure(&failure);
        return TurnResult::ToolError(code.into_turn_failure(
            TurnFailureKind::NonRetryable,
            format!("{}: {summary}", code.as_str()),
        ));
    }
    turn_result_from_plan_failure(failure)
}

fn decide_safe_lane_plan_failure_action(
    failure: PlanRunFailure,
    route: SafeLaneFailureRoute,
    next_plan_start_tool_index: usize,
    next_seed_tool_outputs: Vec<String>,
) -> SafeLaneRoundDecision {
    if route.should_replan() {
        return SafeLaneRoundDecision::Replan {
            reason: summarize_plan_failure(&failure),
            next_plan_start_tool_index,
            next_seed_tool_outputs,
        };
    }

    SafeLaneRoundDecision::Finalize {
        result: terminal_turn_result_from_plan_failure_with_route(failure, route),
    }
}

struct SafeLanePlanNodeExecutor<'a> {
    tool_intents: &'a [ToolIntent],
    session_context: &'a SessionContext,
    app_dispatcher: &'a dyn AppToolDispatcher,
    kernel_ctx: Option<&'a KernelContext>,
    ingress: Option<&'a ConversationIngressContext>,
    verify_output_non_empty: bool,
    tool_outputs: Mutex<Vec<String>>,
    tool_result_payload_summary_limit_chars: usize,
}

impl<'a> SafeLanePlanNodeExecutor<'a> {
    fn new(
        tool_intents: &'a [ToolIntent],
        session_context: &'a SessionContext,
        app_dispatcher: &'a dyn AppToolDispatcher,
        kernel_ctx: Option<&'a KernelContext>,
        ingress: Option<&'a ConversationIngressContext>,
        verify_output_non_empty: bool,
        seed_tool_outputs: Vec<String>,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            tool_intents,
            session_context,
            app_dispatcher,
            kernel_ctx,
            ingress,
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
                    self.session_context,
                    self.app_dispatcher,
                    self.kernel_ctx,
                    self.ingress,
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
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    kernel_ctx: Option<&KernelContext>,
    ingress: Option<&ConversationIngressContext>,
    payload_summary_limit_chars: usize,
) -> Result<String, PlanNodeError> {
    let engine = TurnEngine::with_tool_result_payload_summary_limit(1, payload_summary_limit_chars);
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent.clone()],
        raw_meta: Value::Null,
    };

    match engine
        .execute_turn_in_context(&turn, session_context, app_dispatcher, kernel_ctx, ingress)
        .await
    {
        TurnResult::FinalText(output) => Ok(output),
        TurnResult::NeedsApproval(requirement) => Err(PlanNodeError::policy_denied(
            format_approval_required_reply("", &requirement),
        )),
        TurnResult::ToolDenied(failure) => Err(PlanNodeError::policy_denied(failure.reason)),
        TurnResult::ToolError(failure) => Err(PlanNodeError {
            kind: match failure.kind {
                TurnFailureKind::Retryable => PlanNodeErrorKind::Retryable,
                TurnFailureKind::PolicyDenied
                | TurnFailureKind::NonRetryable
                | TurnFailureKind::Provider => PlanNodeErrorKind::NonRetryable,
            },
            message: failure.reason,
        }),
        TurnResult::ProviderError(failure) => Err(PlanNodeError {
            kind: PlanNodeErrorKind::NonRetryable,
            message: failure.reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::repository::FinalizeSessionTerminalResult;
    use std::path::PathBuf;

    fn unique_sqlite_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loongclaw-turn-coordinator-{label}-{}.sqlite3",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
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
    fn finalize_recovered_child(
        repo: &SessionRepository,
        expected_state: SessionState,
    ) -> FinalizeSessionTerminalResult {
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
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn finalize_async_delegate_spawn_failure_does_not_overwrite_recovered_failure() {
        let memory_config = sqlite_memory_config("recovered-ready-child");
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

    #[test]
    fn provider_turn_session_state_appends_user_input_and_keeps_estimate() {
        let session = ProviderTurnSessionState::from_assembled_context(
            AssembledConversationContext {
                messages: vec![serde_json::json!({
                    "role": "system",
                    "content": "sys"
                })],
                estimated_tokens: Some(42),
                system_prompt_addition: None,
            },
            "hello world",
            None,
        );

        assert_eq!(session.estimated_tokens, Some(42));
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1]["role"], "user");
        assert_eq!(session.messages[1]["content"], "hello world");
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
                estimated_tokens: Some(42),
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
                had_tool_intents: true,
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
            },
            config,
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
    fn scope_provider_turn_tool_intents_populates_missing_ids_and_preserves_existing_ids() {
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

        assert_eq!(scoped.tool_intents[0].session_id, "session-a");
        assert_eq!(scoped.tool_intents[0].turn_id, "turn-a");
        assert_eq!(scoped.tool_intents[1].session_id, "already-session");
        assert_eq!(scoped.tool_intents[1].turn_id, "already-turn");
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
                had_tool_intents: false,
                raw_tool_output_requested: false,
                turn_result: TurnResult::FinalText("hello there".to_owned()),
                safe_lane_terminal_route: None,
            },
            LoongClawConfig::default(),
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
                estimated_tokens: Some(42),
                system_prompt_addition: None,
            },
            "say hello",
            None,
        );
        let resolved = ResolvedProviderTurn::PersistReply(ResolvedProviderReply {
            reply: "done".to_owned(),
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

    #[test]
    fn decide_provider_request_action_inlines_synthetic_reply_when_requested() {
        let decision = decide_provider_turn_request_action(
            Err("provider unavailable".to_owned()),
            ProviderErrorMode::InlineMessage,
        );

        if let ProviderTurnRequestAction::FinalizeInlineProviderError { reply } = decision {
            assert!(reply.contains("provider unavailable"));
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_provider_request_action_returns_error_in_propagate_mode() {
        let decision = decide_provider_turn_request_action(
            Err("provider unavailable".to_owned()),
            ProviderErrorMode::Propagate,
        );

        if let ProviderTurnRequestAction::ReturnError { error } = decision {
            assert_eq!(error, "provider unavailable");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn safe_lane_route_retryable_failure_replans_with_remaining_budget() {
        let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
        let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(1));

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Replan);
        assert_eq!(route.reason, SafeLaneFailureRouteReason::RetryableFailure);
        assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
        assert_eq!(route.reason.as_str(), "retryable_failure");
    }

    #[test]
    fn safe_lane_route_retryable_failure_becomes_terminal_after_budget_exhaustion() {
        let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
        let route = SafeLaneFailureRoute::from_failure(
            &failure,
            SafeLaneReplanBudget::new(1).after_replan(),
        );

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            route.reason,
            SafeLaneFailureRouteReason::RoundBudgetExhausted
        );
        assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
        assert!(route.is_base_round_budget_terminal());
    }

    #[test]
    fn safe_lane_route_policy_denied_failure_is_terminal() {
        let failure = TurnFailure::policy_denied("safe_lane_plan_node_policy_denied", "denied");
        let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(route.reason, SafeLaneFailureRouteReason::PolicyDenied);
        assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    }

    #[test]
    fn safe_lane_route_non_retryable_failure_is_terminal() {
        let failure = TurnFailure::non_retryable("safe_lane_plan_node_non_retryable_error", "bad");
        let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            route.reason,
            SafeLaneFailureRouteReason::NonRetryableFailure
        );
        assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    }

    #[test]
    fn turn_failure_from_plan_failure_node_error_mapping_is_stable() {
        let cases = [
            (
                PlanNodeErrorKind::ApprovalRequired,
                TurnFailureKind::PolicyDenied,
                "safe_lane_plan_node_policy_denied",
                false,
            ),
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
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        };
        let metrics = SafeLaneExecutionMetrics {
            total_attempts_used: 2,
            ..SafeLaneExecutionMetrics::default()
        };
        let guarded =
            route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
        assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            guarded.reason,
            SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
        );
        assert_eq!(
            guarded.source,
            SafeLaneFailureRouteSource::BackpressureGuard
        );
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
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        };
        let metrics = SafeLaneExecutionMetrics {
            replans_triggered: 1,
            ..SafeLaneExecutionMetrics::default()
        };
        let guarded =
            route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
        assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            guarded.reason,
            SafeLaneFailureRouteReason::BackpressureReplansExhausted
        );
        assert_eq!(
            guarded.source,
            SafeLaneFailureRouteSource::BackpressureGuard
        );
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
    fn safe_lane_backpressure_budget_detects_attempt_exhaustion() {
        let budget = SafeLaneBackpressureBudget::new(2, 10);
        let metrics = SafeLaneExecutionMetrics {
            total_attempts_used: 2,
            ..SafeLaneExecutionMetrics::default()
        };

        assert_eq!(
            budget.continuation_decision(metrics.total_attempts_used, metrics.replans_triggered),
            SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            }
        );
    }

    #[test]
    fn decide_safe_lane_failure_route_applies_backpressure_after_retryable_base_route() {
        let mut config = LoongClawConfig::default();
        config.conversation.safe_lane_backpressure_guard_enabled = true;
        config
            .conversation
            .safe_lane_backpressure_max_total_attempts = 2;
        config.conversation.safe_lane_backpressure_max_replans = 10;

        let route = decide_safe_lane_failure_route(
            &config,
            &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
            SafeLaneReplanBudget::new(3),
            SafeLaneExecutionMetrics {
                total_attempts_used: 2,
                ..SafeLaneExecutionMetrics::default()
            },
            SafeLaneSessionGovernorDecision::default(),
        );

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            route.reason,
            SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
        );
        assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
    }

    #[test]
    fn decide_safe_lane_failure_route_applies_session_governor_override_to_exhausted_budget() {
        let config = LoongClawConfig::default();
        let route = decide_safe_lane_failure_route(
            &config,
            &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
            SafeLaneReplanBudget::new(1).after_replan(),
            SafeLaneExecutionMetrics::default(),
            SafeLaneSessionGovernorDecision {
                force_no_replan: true,
                ..SafeLaneSessionGovernorDecision::default()
            },
        );

        assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
        assert_eq!(
            route.reason,
            SafeLaneFailureRouteReason::SessionGovernorNoReplan
        );
        assert_eq!(route.source, SafeLaneFailureRouteSource::SessionGovernor);
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
    fn summarize_governor_history_signals_ignores_unknown_backpressure_like_strings() {
        let contents = [
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"unknown_backpressure_hint","route_reason":"backpressure_noise"}}"#,
        ];

        let signals = summarize_governor_history_signals(contents.iter().copied());
        assert_eq!(signals.final_status_failed_samples, vec![true]);
        assert_eq!(signals.backpressure_failure_samples, vec![false]);
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
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            source: SafeLaneFailureRouteSource::BaseRouting,
        };
        let governor = SafeLaneSessionGovernorDecision {
            force_no_replan: true,
            ..SafeLaneSessionGovernorDecision::default()
        };
        let overridden = route.with_session_governor_override(governor);
        assert_eq!(
            overridden.reason,
            SafeLaneFailureRouteReason::SessionGovernorNoReplan
        );
        assert_eq!(
            overridden.source,
            SafeLaneFailureRouteSource::SessionGovernor
        );
    }

    #[test]
    fn terminal_verify_failure_uses_backpressure_error_code() {
        let failure = terminal_turn_failure_from_verify_failure(
            "retryable verify failure",
            true,
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
                source: SafeLaneFailureRouteSource::BackpressureGuard,
            },
        );
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_backpressure_guard"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    }

    #[test]
    fn safe_lane_terminal_verify_failure_code_prefers_budget_exhaustion_for_retryable_base_route() {
        let code = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            source: SafeLaneFailureRouteSource::BaseRouting,
        }
        .terminal_verify_failure_code(true);
        assert_eq!(code, SafeLaneFailureCode::VerifyFailedBudgetExhausted);
    }

    #[test]
    fn safe_lane_route_verify_summary_label_marks_backpressure_guard() {
        let label = SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        }
        .verify_terminal_summary_label();
        assert_eq!(label, "verify_failed_backpressure_guard");
    }

    #[test]
    fn safe_lane_route_profile_methods_encode_decision_and_source_labels() {
        let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure);
        assert!(route.should_replan());
        assert_eq!(route.decision_label(), "replan");
        assert_eq!(route.source_label(), "base_routing");

        let terminal = SafeLaneFailureRoute::terminal_with_source(
            SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            SafeLaneFailureRouteSource::SessionGovernor,
        );
        assert!(!terminal.should_replan());
        assert_eq!(terminal.decision_label(), "terminal");
        assert_eq!(terminal.source_label(), "session_governor");
    }

    #[test]
    fn safe_lane_route_backpressure_transition_is_localized_on_route() {
        let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure)
            .with_backpressure_guard(
                Some(SafeLaneBackpressureBudget::new(2, 10)),
                SafeLaneExecutionMetrics {
                    total_attempts_used: 2,
                    ..SafeLaneExecutionMetrics::default()
                },
            );
        assert!(!route.should_replan());
        assert_eq!(
            route.reason,
            SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
        );
        assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
    }

    #[test]
    fn terminal_verify_failure_uses_budget_exhaustion_error_code() {
        let failure = terminal_turn_failure_from_verify_failure(
            "retryable verify failure",
            true,
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
                source: SafeLaneFailureRouteSource::BaseRouting,
            },
        );
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_budget_exhausted"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    }

    #[test]
    fn terminal_verify_failure_uses_session_governor_error_code() {
        let failure = terminal_turn_failure_from_verify_failure(
            "retryable verify failure",
            true,
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            },
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
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        };
        assert_eq!(
            route.terminal_plan_failure_code(),
            Some(SafeLaneFailureCode::PlanSessionGovernorNoReplan)
        );
        let result = terminal_turn_result_from_plan_failure_with_route(failure, route);
        let meta = result.failure().expect("failure metadata");
        assert_eq!(meta.code, "safe_lane_plan_session_governor_no_replan");
        assert_eq!(meta.kind, TurnFailureKind::NonRetryable);
    }

    #[test]
    fn decide_safe_lane_verify_failure_action_replans_with_remaining_budget() {
        let decision = decide_safe_lane_verify_failure_action(
            "missing anchors",
            true,
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Replan,
                reason: SafeLaneFailureRouteReason::RetryableFailure,
                source: SafeLaneFailureRouteSource::BaseRouting,
            },
        );

        if let SafeLaneRoundDecision::Replan {
            reason,
            next_plan_start_tool_index,
            next_seed_tool_outputs,
        } = decision
        {
            assert_eq!(reason, "verify_failed");
            assert_eq!(next_plan_start_tool_index, 0);
            assert!(next_seed_tool_outputs.is_empty());
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_safe_lane_verify_failure_action_terminalizes_with_governor_code() {
        let decision = decide_safe_lane_verify_failure_action(
            "missing anchors",
            true,
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            },
        );

        if let SafeLaneRoundDecision::Finalize {
            result: TurnResult::ToolError(failure),
        } = decision
        {
            assert_eq!(
                failure.code,
                "safe_lane_plan_verify_failed_session_governor"
            );
            assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_safe_lane_plan_failure_action_replans_with_failed_subgraph_cursor() {
        let decision = decide_safe_lane_plan_failure_action(
            PlanRunFailure::NodeFailed {
                node_id: "tool-2".to_owned(),
                attempts_used: 1,
                last_error_kind: PlanNodeErrorKind::Retryable,
                last_error: "transient".to_owned(),
            },
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Replan,
                reason: SafeLaneFailureRouteReason::RetryableFailure,
                source: SafeLaneFailureRouteSource::BaseRouting,
            },
            1,
            vec!["[ok] {\"path\":\"note.md\"}".to_owned()],
        );

        if let SafeLaneRoundDecision::Replan {
            reason,
            next_plan_start_tool_index,
            next_seed_tool_outputs,
        } = decision
        {
            assert_eq!(
                reason,
                "node_failed node=tool-2 error_kind=Retryable reason=transient"
            );
            assert_eq!(next_plan_start_tool_index, 1);
            assert_eq!(next_seed_tool_outputs.len(), 1);
            assert!(next_seed_tool_outputs[0].contains("note.md"));
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_safe_lane_plan_failure_action_terminalizes_with_backpressure_code() {
        let decision = decide_safe_lane_plan_failure_action(
            PlanRunFailure::NodeFailed {
                node_id: "tool-1".to_owned(),
                attempts_used: 2,
                last_error_kind: PlanNodeErrorKind::Retryable,
                last_error: "transient".to_owned(),
            },
            SafeLaneFailureRoute {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
                source: SafeLaneFailureRouteSource::BackpressureGuard,
            },
            0,
            Vec::new(),
        );

        if let SafeLaneRoundDecision::Finalize {
            result: TurnResult::ToolError(failure),
        } = decision
        {
            assert_eq!(failure.code, "safe_lane_plan_backpressure_guard");
            assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }
}
