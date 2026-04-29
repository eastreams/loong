use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ContextCompactionDiagnostics;
use super::prompt_fragments::PromptFrameLayer;
use super::prompt_frame::PromptFrameSummary;
use super::safe_lane_failure::{
    SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
    SafeLaneTerminalRouteSnapshot, is_safe_lane_backpressure_failure_code,
    is_safe_lane_backpressure_route_reason,
};
use super::turn_budget::SafeLaneFailureRouteReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFinalStatus {
    Succeeded,
    Failed,
}

impl SafeLaneFinalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneMetricsSnapshot {
    pub rounds_started: u32,
    pub rounds_succeeded: u32,
    pub rounds_failed: u32,
    pub verify_failures: u32,
    pub replans_triggered: u32,
    pub total_attempts_used: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneToolOutputSnapshot {
    pub output_lines: u32,
    pub result_lines: u32,
    pub truncated_result_lines: u32,
    pub any_truncated: bool,
    pub truncation_ratio_milli: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneHealthSignalSnapshot {
    pub severity: String,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneEventSummary {
    pub lane_selected_events: u32,
    pub round_started_events: u32,
    pub round_completed_succeeded_events: u32,
    pub round_completed_failed_events: u32,
    pub verify_failed_events: u32,
    pub verify_policy_adjusted_events: u32,
    pub replan_triggered_events: u32,
    pub final_status_events: u32,
    pub session_governor_engaged_events: u32,
    pub session_governor_force_no_replan_events: u32,
    pub session_governor_failed_threshold_triggered_events: u32,
    pub session_governor_backpressure_threshold_triggered_events: u32,
    pub session_governor_trend_threshold_triggered_events: u32,
    pub session_governor_recovery_threshold_triggered_events: u32,
    pub session_governor_metrics_snapshots_seen: u32,
    pub session_governor_latest_trend_samples: Option<u32>,
    pub session_governor_latest_trend_min_samples: Option<u32>,
    pub session_governor_latest_trend_failure_ewma_milli: Option<u32>,
    pub session_governor_latest_trend_backpressure_ewma_milli: Option<u32>,
    pub session_governor_latest_recovery_success_streak: Option<u32>,
    pub session_governor_latest_recovery_success_streak_threshold: Option<u32>,
    pub final_status: Option<SafeLaneFinalStatus>,
    pub final_failure_code: Option<String>,
    pub final_route_decision: Option<String>,
    pub final_route_reason: Option<String>,
    pub latest_metrics: Option<SafeLaneMetricsSnapshot>,
    pub latest_tool_output: Option<SafeLaneToolOutputSnapshot>,
    pub metrics_snapshots_seen: u32,
    pub tool_output_snapshots_seen: u32,
    pub tool_output_truncated_events: u32,
    pub tool_output_result_lines_total: u64,
    pub tool_output_truncated_result_lines_total: u64,
    pub tool_output_aggregate_truncation_ratio_milli: Option<u32>,
    pub tool_output_truncation_verify_failed_events: u32,
    pub tool_output_truncation_replan_events: u32,
    pub tool_output_truncation_final_failure_events: u32,
    pub latest_health_signal: Option<SafeLaneHealthSignalSnapshot>,
    pub health_signal_snapshots_seen: u32,
    pub health_signal_warn_events: u32,
    pub health_signal_critical_events: u32,
    pub route_decision_counts: BTreeMap<String, u32>,
    pub route_reason_counts: BTreeMap<String, u32>,
    pub failure_code_counts: BTreeMap<String, u32>,
    pub final_status_counts: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SafeLaneHistoryProjection {
    pub(crate) summary: SafeLaneEventSummary,
    pub(crate) final_status_failed_samples: Vec<bool>,
    pub(crate) backpressure_failure_samples: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SafeLaneFinalStatusSample {
    failed: bool,
    backpressure: bool,
}

impl SafeLaneEventSummary {
    pub fn typed_final_failure_code(&self) -> Option<SafeLaneFailureCode> {
        self.final_failure_code
            .as_deref()
            .and_then(SafeLaneFailureCode::parse)
    }

    pub fn typed_final_route_reason(&self) -> Option<SafeLaneFailureRouteReason> {
        self.final_route_reason
            .as_deref()
            .and_then(SafeLaneFailureRouteReason::parse)
    }

    pub fn final_status_events_for(&self, status: SafeLaneFinalStatus) -> u32 {
        self.final_status_counts
            .get(status.as_str())
            .copied()
            .unwrap_or_default()
    }

    pub fn failed_final_status_events(&self) -> u32 {
        self.final_status_events_for(SafeLaneFinalStatus::Failed)
    }

    pub fn backpressure_failure_events(&self) -> u32 {
        self.failure_code_counts
            .iter()
            .filter_map(|(failure_code, count)| {
                SafeLaneFailureCode::parse(failure_code)
                    .filter(|code| code.is_backpressure())
                    .map(|_| *count)
            })
            .sum()
    }

    pub fn backpressure_route_reason_events(&self) -> u32 {
        self.route_reason_counts
            .iter()
            .filter_map(|(route_reason, count)| {
                SafeLaneFailureRouteReason::parse(route_reason)
                    .filter(|reason| reason.is_backpressure())
                    .map(|_| *count)
            })
            .sum()
    }

    pub fn has_terminal_instability_final_failure(&self) -> bool {
        matches!(self.final_status, Some(SafeLaneFinalStatus::Failed))
            && self
                .typed_final_failure_code()
                .map(SafeLaneFailureCode::is_terminal_instability)
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCheckpointStage {
    PostPersist,
    Finalized,
    FinalizationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCheckpointProgressStatus {
    Pending,
    Skipped,
    Completed,
    Failed,
    FailedOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCheckpointFailureStep {
    AfterTurn,
    Compaction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCheckpointSessionState {
    #[default]
    NotDurable,
    PendingFinalization,
    Finalized,
    FinalizationFailed,
}

impl TurnCheckpointSessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotDurable => "not_durable",
            Self::PendingFinalization => "pending_finalization",
            Self::Finalized => "finalized",
            Self::FinalizationFailed => "finalization_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCheckpointRecoveryAction {
    #[default]
    None,
    RunAfterTurn,
    RunCompaction,
    RunAfterTurnAndCompaction,
    InspectManually,
}

impl TurnCheckpointRecoveryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RunAfterTurn => "run_after_turn",
            Self::RunCompaction => "run_compaction",
            Self::RunAfterTurnAndCompaction => "run_after_turn_and_compaction",
            Self::InspectManually => "inspect_manually",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCheckpointRepairManualReason {
    CheckpointIdentityMissing,
    SafeLaneBackpressureTerminalRequiresManualInspection,
    SafeLaneSessionGovernorTerminalRequiresManualInspection,
    CheckpointStateRequiresManualInspection,
}

impl TurnCheckpointRepairManualReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CheckpointIdentityMissing => "checkpoint_identity_missing",
            Self::SafeLaneBackpressureTerminalRequiresManualInspection => {
                "safe_lane_backpressure_terminal_requires_manual_inspection"
            }
            Self::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                "safe_lane_session_governor_terminal_requires_manual_inspection"
            }
            Self::CheckpointStateRequiresManualInspection => {
                "checkpoint_state_requires_manual_inspection"
            }
        }
    }

    pub fn from_safe_lane_terminal_route(route: SafeLaneTerminalRouteSnapshot) -> Option<Self> {
        if route.is_backpressure_override_terminal() {
            return Some(Self::SafeLaneBackpressureTerminalRequiresManualInspection);
        }
        if route.is_session_governor_override_terminal() {
            return Some(Self::SafeLaneSessionGovernorTerminalRequiresManualInspection);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnCheckpointRepairPlan {
    action: TurnCheckpointRecoveryAction,
    manual_reason: Option<TurnCheckpointRepairManualReason>,
    after_turn_status: TurnCheckpointProgressStatus,
    compaction_status: TurnCheckpointProgressStatus,
}

impl TurnCheckpointRepairPlan {
    fn new(
        action: TurnCheckpointRecoveryAction,
        manual_reason: Option<TurnCheckpointRepairManualReason>,
        after_turn_status: TurnCheckpointProgressStatus,
        compaction_status: TurnCheckpointProgressStatus,
    ) -> Self {
        Self {
            action,
            manual_reason,
            after_turn_status,
            compaction_status,
        }
    }

    pub fn action(self) -> TurnCheckpointRecoveryAction {
        self.action
    }

    pub fn manual_reason(self) -> Option<TurnCheckpointRepairManualReason> {
        self.manual_reason
    }

    pub fn should_run_after_turn(self) -> bool {
        matches!(
            self.action,
            TurnCheckpointRecoveryAction::RunAfterTurn
                | TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction
        )
    }

    pub fn should_run_compaction(self) -> bool {
        matches!(
            self.action,
            TurnCheckpointRecoveryAction::RunCompaction
                | TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction
        )
    }

    pub fn after_turn_status(self) -> TurnCheckpointProgressStatus {
        self.after_turn_status
    }

    pub fn compaction_status(self) -> TurnCheckpointProgressStatus {
        self.compaction_status
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpointEventSummary {
    pub checkpoint_events: u32,
    pub post_persist_events: u32,
    pub finalized_events: u32,
    pub finalization_failed_events: u32,
    pub latest_schema_version: Option<u32>,
    pub latest_stage: Option<TurnCheckpointStage>,
    pub latest_after_turn: Option<TurnCheckpointProgressStatus>,
    pub latest_compaction: Option<TurnCheckpointProgressStatus>,
    pub latest_failure_step: Option<TurnCheckpointFailureStep>,
    pub latest_failure_error: Option<String>,
    pub latest_lane: Option<String>,
    pub latest_result_kind: Option<String>,
    pub latest_persistence_mode: Option<String>,
    pub latest_safe_lane_terminal_route: Option<SafeLaneTerminalRouteSnapshot>,
    pub latest_identity_present: Option<bool>,
    pub latest_runs_after_turn: Option<bool>,
    pub latest_attempts_context_compaction: Option<bool>,
    pub latest_compaction_diagnostics: Option<ContextCompactionDiagnostics>,
    pub stage_counts: BTreeMap<String, u32>,
    pub session_state: TurnCheckpointSessionState,
    pub checkpoint_durable: bool,
    pub requires_recovery: bool,
    pub reply_durable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TurnCheckpointHistoryProjection {
    pub(crate) summary: TurnCheckpointEventSummary,
    pub(crate) latest_checkpoint: Option<Value>,
}

impl TurnCheckpointEventSummary {
    pub fn latest_safe_lane_route_decision_label(&self) -> Option<&'static str> {
        self.latest_safe_lane_terminal_route
            .map(SafeLaneTerminalRouteSnapshot::decision_label)
    }

    pub fn latest_safe_lane_route_reason_label(&self) -> Option<&'static str> {
        self.latest_safe_lane_terminal_route
            .map(SafeLaneTerminalRouteSnapshot::reason_label)
    }

    pub fn latest_safe_lane_route_source_label(&self) -> Option<&'static str> {
        self.latest_safe_lane_terminal_route
            .map(SafeLaneTerminalRouteSnapshot::source_label)
    }

    pub fn latest_safe_lane_route_labels_or_default(
        &self,
    ) -> (&'static str, &'static str, &'static str) {
        (
            self.latest_safe_lane_route_decision_label().unwrap_or("-"),
            self.latest_safe_lane_route_reason_label().unwrap_or("-"),
            self.latest_safe_lane_route_source_label().unwrap_or("-"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationEventRecord {
    pub event: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryFirstEventSummary {
    pub search_round_events: u32,
    pub followup_requested_events: u32,
    pub followup_result_events: u32,
    pub raw_output_followup_events: u32,
    pub search_to_invoke_hits: u32,
    pub aggregate_added_estimated_tokens: u64,
    pub added_estimated_token_samples: u32,
    pub average_added_estimated_tokens: Option<u32>,
    pub latest_followup_outcome: Option<String>,
    pub latest_followup_tool_name: Option<String>,
    pub latest_followup_target_tool_id: Option<String>,
    pub latest_initial_estimated_tokens: Option<u32>,
    pub latest_followup_estimated_tokens: Option<u32>,
    pub latest_added_estimated_tokens: Option<u32>,
    pub outcome_counts: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFrameEventSummary {
    pub snapshot_events: u32,
    pub initial_snapshot_events: u32,
    pub followup_snapshot_events: u32,
    pub stable_runtime_hash_change_events: u32,
    pub session_latched_hash_change_events: u32,
    pub stable_prefix_hash_change_events: u32,
    pub cached_prefix_hash_change_events: u32,
    pub advisory_profile_hash_change_events: u32,
    pub session_local_recall_hash_change_events: u32,
    pub recent_window_hash_change_events: u32,
    pub turn_ephemeral_hash_change_events: u32,
    pub latest_phase: Option<String>,
    pub latest_total_estimated_tokens: Option<usize>,
    pub latest_stable_runtime_segment_count: Option<usize>,
    pub latest_stable_runtime_estimated_tokens: Option<usize>,
    pub latest_session_latched_segment_count: Option<usize>,
    pub latest_session_latched_estimated_tokens: Option<usize>,
    pub latest_advisory_profile_segment_count: Option<usize>,
    pub latest_advisory_profile_estimated_tokens: Option<usize>,
    pub latest_session_local_recall_segment_count: Option<usize>,
    pub latest_session_local_recall_estimated_tokens: Option<usize>,
    pub latest_recent_window_segment_count: Option<usize>,
    pub latest_recent_window_estimated_tokens: Option<usize>,
    pub latest_turn_ephemeral_segment_count: Option<usize>,
    pub latest_turn_ephemeral_estimated_tokens: Option<usize>,
    pub latest_stable_runtime_hash: Option<String>,
    pub latest_session_latched_hash: Option<String>,
    pub latest_stable_prefix_hash: Option<String>,
    pub latest_cached_prefix_hash: Option<String>,
    pub latest_advisory_profile_hash: Option<String>,
    pub latest_session_local_recall_hash: Option<String>,
    pub latest_recent_window_hash: Option<String>,
    pub latest_turn_ephemeral_hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FastLaneToolBatchSegmentSnapshot {
    pub segment_index: u32,
    pub scheduling_class: String,
    pub execution_mode: String,
    pub intent_count: u32,
    pub observed_peak_in_flight: Option<u32>,
    pub observed_wall_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FastLaneToolBatchEventSummary {
    pub batch_events: u32,
    pub parallel_execution_enabled_batches: u32,
    pub parallel_only_batches: u32,
    pub mixed_execution_batches: u32,
    pub sequential_only_batches: u32,
    pub batches_without_segments: u32,
    pub total_intents_seen: u64,
    pub total_parallel_safe_intents_seen: u64,
    pub total_serial_only_intents_seen: u64,
    pub total_parallel_segments_seen: u64,
    pub total_sequential_segments_seen: u64,
    pub parallel_execution_max_in_flight_samples: u32,
    pub parallel_execution_max_in_flight_sum: u64,
    pub parallel_execution_max_in_flight_max: Option<u32>,
    pub observed_peak_in_flight_samples: u32,
    pub observed_peak_in_flight_sum: u64,
    pub observed_peak_in_flight_max: Option<u32>,
    pub observed_wall_time_ms_samples: u32,
    pub observed_wall_time_ms_sum: u64,
    pub observed_wall_time_ms_max: Option<u64>,
    pub degraded_parallel_segments: u64,
    pub scheduling_class_counts: BTreeMap<String, u32>,
    pub execution_mode_counts: BTreeMap<String, u32>,
    pub latest_schema_version: Option<u32>,
    pub latest_total_intents: Option<u32>,
    pub latest_parallel_execution_enabled: Option<bool>,
    pub latest_parallel_execution_max_in_flight: Option<u32>,
    pub latest_observed_peak_in_flight: Option<u32>,
    pub latest_observed_wall_time_ms: Option<u64>,
    pub latest_parallel_safe_intents: Option<u32>,
    pub latest_serial_only_intents: Option<u32>,
    pub latest_parallel_segments: Option<u32>,
    pub latest_sequential_segments: Option<u32>,
    pub latest_segments: Vec<FastLaneToolBatchSegmentSnapshot>,
}

pub fn parse_conversation_event(content: &str) -> Option<ConversationEventRecord> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    if parsed.get("type")?.as_str()? != "conversation_event" {
        return None;
    }
    let event = parsed.get("event")?.as_str()?.to_owned();
    let payload = parsed.get("payload").cloned().unwrap_or(Value::Null);
    Some(ConversationEventRecord { event, payload })
}

pub(crate) fn summarize_safe_lane_history<'a, I>(contents: I) -> SafeLaneHistoryProjection
where
    I: IntoIterator<Item = &'a str>,
{
    let mut projection = SafeLaneHistoryProjection::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        if let Some(sample) = fold_safe_lane_event_record(&record, &mut projection.summary) {
            projection.final_status_failed_samples.push(sample.failed);
            projection
                .backpressure_failure_samples
                .push(sample.backpressure);
        }
    }

    projection
}

pub fn summarize_safe_lane_events<'a, I>(contents: I) -> SafeLaneEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    summarize_safe_lane_history(contents).summary
}

pub fn summarize_discovery_first_events<'a, I>(contents: I) -> DiscoveryFirstEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = DiscoveryFirstEventSummary::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        fold_discovery_first_event_record(&record, &mut summary);
    }

    summary
}

pub fn summarize_prompt_frame_events<'a, I>(contents: I) -> PromptFrameEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = PromptFrameEventSummary::default();
    let mut previous_stable_runtime_hash: Option<String> = None;
    let mut previous_session_latched_hash: Option<String> = None;
    let mut previous_stable_prefix_hash: Option<String> = None;
    let mut previous_cached_prefix_hash: Option<String> = None;
    let mut previous_advisory_profile_hash: Option<String> = None;
    let mut previous_session_local_recall_hash: Option<String> = None;
    let mut previous_recent_window_hash: Option<String> = None;
    let mut previous_turn_ephemeral_hash: Option<String> = None;

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        fold_prompt_frame_event_record(
            &record,
            &mut summary,
            &mut previous_stable_runtime_hash,
            &mut previous_session_latched_hash,
            &mut previous_stable_prefix_hash,
            &mut previous_cached_prefix_hash,
            &mut previous_advisory_profile_hash,
            &mut previous_session_local_recall_hash,
            &mut previous_recent_window_hash,
            &mut previous_turn_ephemeral_hash,
        );
    }

    summary
}

pub fn summarize_fast_lane_tool_batch_events<'a, I>(contents: I) -> FastLaneToolBatchEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = FastLaneToolBatchEventSummary::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        fold_fast_lane_tool_batch_event_record(&record, &mut summary);
    }

    summary
}

fn fold_safe_lane_event_record(
    record: &ConversationEventRecord,
    summary: &mut SafeLaneEventSummary,
) -> Option<SafeLaneFinalStatusSample> {
    if !is_safe_lane_event_name(record.event.as_str()) {
        return None;
    }

    let event_name = record.event.as_str();
    let final_status_sample = if event_name == "final_status" {
        match record.payload.get("status").and_then(Value::as_str) {
            Some("failed") => Some(SafeLaneFinalStatusSample {
                failed: true,
                backpressure: is_backpressure_safe_lane_final_status_payload(&record.payload),
            }),
            Some("succeeded") => Some(SafeLaneFinalStatusSample {
                failed: false,
                backpressure: false,
            }),
            _ => None,
        }
    } else {
        None
    };
    let final_status_is_failed = final_status_sample
        .map(|sample| sample.failed)
        .unwrap_or(false);

    match event_name {
        "lane_selected" => {
            summary.lane_selected_events = summary.lane_selected_events.saturating_add(1);
        }
        "plan_round_started" => {
            summary.round_started_events = summary.round_started_events.saturating_add(1);
        }
        "plan_round_completed" => {
            let is_succeeded = record
                .payload
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status == "succeeded")
                .unwrap_or(false);
            if is_succeeded {
                summary.round_completed_succeeded_events =
                    summary.round_completed_succeeded_events.saturating_add(1);
            } else {
                summary.round_completed_failed_events =
                    summary.round_completed_failed_events.saturating_add(1);
            }
        }
        "verify_failed" => {
            summary.verify_failed_events = summary.verify_failed_events.saturating_add(1);
        }
        "verify_policy_adjusted" => {
            summary.verify_policy_adjusted_events =
                summary.verify_policy_adjusted_events.saturating_add(1);
        }
        "replan_triggered" => {
            summary.replan_triggered_events = summary.replan_triggered_events.saturating_add(1);
        }
        "final_status" => {
            summary.final_status_events = summary.final_status_events.saturating_add(1);
            match record.payload.get("status").and_then(Value::as_str) {
                Some("succeeded") => {
                    summary.final_status = Some(SafeLaneFinalStatus::Succeeded);
                    bump_count(
                        &mut summary.final_status_counts,
                        SafeLaneFinalStatus::Succeeded.as_str(),
                    );
                }
                Some("failed") => {
                    summary.final_status = Some(SafeLaneFinalStatus::Failed);
                    bump_count(
                        &mut summary.final_status_counts,
                        SafeLaneFinalStatus::Failed.as_str(),
                    );
                }
                _ => {}
            }
            summary.final_failure_code = record
                .payload
                .get("failure_code")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            summary.final_route_decision = record
                .payload
                .get("route_decision")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            summary.final_route_reason = record
                .payload
                .get("route_reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        _ => {}
    }

    if let Some(route_decision) = record
        .payload
        .get("route_decision")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        bump_count(&mut summary.route_decision_counts, route_decision);
    }
    if let Some(failure_code) = record
        .payload
        .get("failure_code")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        bump_count(&mut summary.failure_code_counts, failure_code);
    }
    if let Some(route_reason) = record
        .payload
        .get("route_reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        bump_count(&mut summary.route_reason_counts, route_reason);
    }
    fold_session_governor_summary(record.payload.get("session_governor"), summary);

    if let Some(metrics) = parse_metrics_snapshot(record.payload.get("metrics")) {
        summary.metrics_snapshots_seen = summary.metrics_snapshots_seen.saturating_add(1);
        summary.latest_metrics = Some(metrics);
    }
    if let Some(tool_output) = parse_tool_output_snapshot(record.payload.get("tool_output_stats")) {
        summary.tool_output_snapshots_seen = summary.tool_output_snapshots_seen.saturating_add(1);
        if tool_output.any_truncated || tool_output.truncated_result_lines > 0 {
            summary.tool_output_truncated_events =
                summary.tool_output_truncated_events.saturating_add(1);
            if event_name == "verify_failed" {
                summary.tool_output_truncation_verify_failed_events = summary
                    .tool_output_truncation_verify_failed_events
                    .saturating_add(1);
            }
            if event_name == "replan_triggered" {
                summary.tool_output_truncation_replan_events = summary
                    .tool_output_truncation_replan_events
                    .saturating_add(1);
            }
            if final_status_is_failed {
                summary.tool_output_truncation_final_failure_events = summary
                    .tool_output_truncation_final_failure_events
                    .saturating_add(1);
            }
        }
        summary.tool_output_result_lines_total = summary
            .tool_output_result_lines_total
            .saturating_add(tool_output.result_lines as u64);
        summary.tool_output_truncated_result_lines_total = summary
            .tool_output_truncated_result_lines_total
            .saturating_add(tool_output.truncated_result_lines as u64);
        summary.tool_output_aggregate_truncation_ratio_milli = compute_truncation_ratio_milli(
            summary.tool_output_truncated_result_lines_total,
            summary.tool_output_result_lines_total,
        );
        summary.latest_tool_output = Some(tool_output);
    }
    if let Some(health_signal) = parse_health_signal_snapshot(record.payload.get("health_signal")) {
        summary.health_signal_snapshots_seen =
            summary.health_signal_snapshots_seen.saturating_add(1);
        match health_signal.severity.as_str() {
            "warn" => {
                summary.health_signal_warn_events =
                    summary.health_signal_warn_events.saturating_add(1);
            }
            "critical" => {
                summary.health_signal_critical_events =
                    summary.health_signal_critical_events.saturating_add(1);
            }
            _ => {}
        }
        summary.latest_health_signal = Some(health_signal);
    }

    final_status_sample
}

fn fold_discovery_first_event_record(
    record: &ConversationEventRecord,
    summary: &mut DiscoveryFirstEventSummary,
) {
    if !is_discovery_first_event_name(record.event.as_str()) {
        return;
    }

    match record.event.as_str() {
        "discovery_first_search_round" => {
            if record
                .payload
                .get("search_tool_calls")
                .and_then(Value::as_u64)
                .is_some()
            {
                summary.search_round_events = summary.search_round_events.saturating_add(1);
            }
            if let Some(initial_estimated_tokens) = record
                .payload
                .get("initial_estimated_tokens")
                .and_then(Value::as_u64)
                .map(|value| value.min(u32::MAX as u64) as u32)
            {
                summary.latest_initial_estimated_tokens = Some(initial_estimated_tokens);
            }
        }
        "discovery_first_followup_requested" => {
            summary.followup_requested_events = summary.followup_requested_events.saturating_add(1);
            if let Some(initial_estimated_tokens) = record
                .payload
                .get("initial_estimated_tokens")
                .and_then(Value::as_u64)
                .map(|value| value.min(u32::MAX as u64) as u32)
            {
                summary.latest_initial_estimated_tokens = Some(initial_estimated_tokens);
            }
            if let Some(followup_estimated_tokens) = record
                .payload
                .get("followup_estimated_tokens")
                .and_then(Value::as_u64)
                .map(|value| value.min(u32::MAX as u64) as u32)
            {
                summary.latest_followup_estimated_tokens = Some(followup_estimated_tokens);
            }
            if let Some(added_estimated_tokens) = record
                .payload
                .get("followup_added_estimated_tokens")
                .and_then(Value::as_u64)
                .map(|value| value.min(u32::MAX as u64) as u32)
            {
                summary.latest_added_estimated_tokens = Some(added_estimated_tokens);
                summary.aggregate_added_estimated_tokens = summary
                    .aggregate_added_estimated_tokens
                    .saturating_add(u64::from(added_estimated_tokens));
                summary.added_estimated_token_samples =
                    summary.added_estimated_token_samples.saturating_add(1);
                summary.average_added_estimated_tokens = Some(
                    summary
                        .aggregate_added_estimated_tokens
                        .saturating_div(u64::from(summary.added_estimated_token_samples))
                        .min(u32::MAX as u64) as u32,
                );
            }
        }
        "discovery_first_followup_result" => {
            let outcome = record.payload.get("outcome").and_then(Value::as_str);
            if outcome.is_some() {
                summary.followup_result_events = summary.followup_result_events.saturating_add(1);
            }

            if record
                .payload
                .get("raw_tool_output_requested")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                summary.raw_output_followup_events =
                    summary.raw_output_followup_events.saturating_add(1);
            }
            if record
                .payload
                .get("resolved_to_tool_invoke")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                summary.search_to_invoke_hits = summary.search_to_invoke_hits.saturating_add(1);
            }

            if let Some(outcome) = outcome {
                summary.latest_followup_outcome = Some(outcome.to_owned());
                bump_count(&mut summary.outcome_counts, outcome);
            }
            summary.latest_followup_tool_name = record
                .payload
                .get("followup_tool_name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            summary.latest_followup_target_tool_id = record
                .payload
                .get("followup_target_tool_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        _ => {}
    }
}

fn fold_prompt_frame_event_record(
    record: &ConversationEventRecord,
    summary: &mut PromptFrameEventSummary,
    previous_stable_runtime_hash: &mut Option<String>,
    previous_session_latched_hash: &mut Option<String>,
    previous_stable_prefix_hash: &mut Option<String>,
    previous_cached_prefix_hash: &mut Option<String>,
    previous_advisory_profile_hash: &mut Option<String>,
    previous_session_local_recall_hash: &mut Option<String>,
    previous_recent_window_hash: &mut Option<String>,
    previous_turn_ephemeral_hash: &mut Option<String>,
) {
    if record.event.as_str() != "provider_prompt_frame_snapshot" {
        return;
    }

    let phase = record.payload.get("phase").and_then(Value::as_str);
    let prompt_frame_value = record.payload.get("prompt_frame").cloned();
    let Some(prompt_frame_value) = prompt_frame_value else {
        return;
    };
    let Ok(prompt_frame_summary) = serde_json::from_value::<PromptFrameSummary>(prompt_frame_value)
    else {
        return;
    };

    summary.snapshot_events = summary.snapshot_events.saturating_add(1);
    match phase {
        Some("initial") => {
            summary.initial_snapshot_events = summary.initial_snapshot_events.saturating_add(1);
        }
        Some("followup") => {
            summary.followup_snapshot_events = summary.followup_snapshot_events.saturating_add(1);
        }
        _ => {}
    }

    summary.latest_phase = phase.map(str::to_owned);
    let stable_runtime_bucket =
        prompt_frame_summary.bucket(PromptFrameLayer::StableRuntimeGuidance);
    let session_latched_bucket =
        prompt_frame_summary.bucket(PromptFrameLayer::SessionLatchedContext);
    let advisory_profile_bucket = prompt_frame_summary.bucket(PromptFrameLayer::AdvisoryProfile);
    let session_local_recall_bucket =
        prompt_frame_summary.bucket(PromptFrameLayer::SessionLocalRecall);
    let recent_window_bucket = prompt_frame_summary.bucket(PromptFrameLayer::RecentWindow);
    let turn_ephemeral_bucket = prompt_frame_summary.bucket(PromptFrameLayer::TurnEphemeralTail);

    summary.latest_total_estimated_tokens =
        Some(prompt_frame_total_estimated_tokens(&prompt_frame_summary));
    summary.latest_stable_runtime_segment_count = stable_runtime_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(prompt_frame_summary.stable_runtime_segment_count));
    summary.latest_stable_runtime_estimated_tokens = stable_runtime_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(prompt_frame_summary.stable_runtime_estimated_tokens));
    summary.latest_session_latched_segment_count = session_latched_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(prompt_frame_summary.session_latched_segment_count));
    summary.latest_session_latched_estimated_tokens = session_latched_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(prompt_frame_summary.session_latched_estimated_tokens));
    summary.latest_advisory_profile_segment_count = advisory_profile_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(prompt_frame_summary.advisory_profile_segment_count));
    summary.latest_advisory_profile_estimated_tokens = advisory_profile_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(prompt_frame_summary.advisory_profile_estimated_tokens));
    summary.latest_session_local_recall_segment_count = session_local_recall_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(
            prompt_frame_summary.session_local_recall_segment_count,
        ));
    summary.latest_session_local_recall_estimated_tokens = session_local_recall_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(
            prompt_frame_summary.session_local_recall_estimated_tokens,
        ));
    summary.latest_recent_window_segment_count = recent_window_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(prompt_frame_summary.recent_window_segment_count));
    summary.latest_recent_window_estimated_tokens = recent_window_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(prompt_frame_summary.recent_window_estimated_tokens));
    summary.latest_turn_ephemeral_segment_count = turn_ephemeral_bucket
        .map(prompt_frame_bucket_segment_count)
        .or(Some(prompt_frame_summary.turn_ephemeral_segment_count));
    summary.latest_turn_ephemeral_estimated_tokens = turn_ephemeral_bucket
        .map(prompt_frame_bucket_estimated_tokens)
        .or(Some(prompt_frame_summary.turn_ephemeral_estimated_tokens));
    summary.latest_stable_runtime_hash = stable_runtime_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.stable_runtime_hash.clone());
    summary.latest_session_latched_hash = session_latched_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.session_latched_hash.clone());
    summary.latest_stable_prefix_hash = prompt_frame_summary.stable_prefix_hash_sha256.clone();
    summary.latest_cached_prefix_hash = prompt_frame_summary.cached_prefix_sha256.clone();
    summary.latest_advisory_profile_hash = advisory_profile_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.advisory_profile_hash.clone());
    summary.latest_session_local_recall_hash = session_local_recall_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.session_local_recall_hash.clone());
    summary.latest_recent_window_hash = recent_window_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.recent_window_hash.clone());
    summary.latest_turn_ephemeral_hash = turn_ephemeral_bucket
        .and_then(prompt_frame_bucket_hash)
        .or_else(|| prompt_frame_summary.turn_ephemeral_hash.clone());

    update_hash_change_count(
        previous_stable_runtime_hash,
        summary.latest_stable_runtime_hash.as_ref(),
        &mut summary.stable_runtime_hash_change_events,
    );
    update_hash_change_count(
        previous_session_latched_hash,
        summary.latest_session_latched_hash.as_ref(),
        &mut summary.session_latched_hash_change_events,
    );
    update_hash_change_count(
        previous_stable_prefix_hash,
        summary.latest_stable_prefix_hash.as_ref(),
        &mut summary.stable_prefix_hash_change_events,
    );
    update_hash_change_count(
        previous_cached_prefix_hash,
        summary.latest_cached_prefix_hash.as_ref(),
        &mut summary.cached_prefix_hash_change_events,
    );
    update_hash_change_count(
        previous_advisory_profile_hash,
        summary.latest_advisory_profile_hash.as_ref(),
        &mut summary.advisory_profile_hash_change_events,
    );
    update_hash_change_count(
        previous_session_local_recall_hash,
        summary.latest_session_local_recall_hash.as_ref(),
        &mut summary.session_local_recall_hash_change_events,
    );
    update_hash_change_count(
        previous_recent_window_hash,
        summary.latest_recent_window_hash.as_ref(),
        &mut summary.recent_window_hash_change_events,
    );
    update_hash_change_count(
        previous_turn_ephemeral_hash,
        summary.latest_turn_ephemeral_hash.as_ref(),
        &mut summary.turn_ephemeral_hash_change_events,
    );
}

fn update_hash_change_count(
    previous_hash: &mut Option<String>,
    current_hash: Option<&String>,
    change_count: &mut u32,
) {
    let current_hash = current_hash.cloned();
    let changed = previous_hash
        .as_ref()
        .zip(current_hash.as_ref())
        .map(|(previous_hash, current_hash)| previous_hash != current_hash)
        .unwrap_or(false);
    if changed {
        *change_count = change_count.saturating_add(1);
    }

    if let Some(current_hash) = current_hash {
        *previous_hash = Some(current_hash);
    }
}

fn prompt_frame_total_estimated_tokens(summary: &PromptFrameSummary) -> usize {
    let total_from_summary = summary.total_estimated_tokens;
    if let Some(total_from_summary) = total_from_summary {
        return total_from_summary;
    }

    let mut total = 0usize;

    for bucket in &summary.buckets {
        let bucket_tokens = prompt_frame_bucket_estimated_tokens(bucket);
        total = total.saturating_add(bucket_tokens);
    }

    total
}

fn prompt_frame_bucket_segment_count(
    bucket: &super::prompt_frame::PromptFrameBucketSummary,
) -> usize {
    let fragment_count = bucket.fragment_count;
    let message_count = bucket.message_count;

    fragment_count.saturating_add(message_count)
}

fn prompt_frame_bucket_estimated_tokens(
    bucket: &super::prompt_frame::PromptFrameBucketSummary,
) -> usize {
    let adjusted_chars = bucket.content_chars.saturating_add(3);

    adjusted_chars / 4
}

fn prompt_frame_bucket_hash(
    bucket: &super::prompt_frame::PromptFrameBucketSummary,
) -> Option<String> {
    bucket.content_sha256.clone()
}

fn fold_fast_lane_tool_batch_event_record(
    record: &ConversationEventRecord,
    summary: &mut FastLaneToolBatchEventSummary,
) {
    if record.event != "fast_lane_tool_batch" {
        return;
    }

    summary.batch_events = summary.batch_events.saturating_add(1);
    summary.latest_schema_version = read_u32_opt(&record.payload, "schema_version");

    let total_intents = read_u32_opt(&record.payload, "total_intents");
    summary.latest_total_intents = total_intents;
    summary.total_intents_seen = summary
        .total_intents_seen
        .saturating_add(total_intents.map(u64::from).unwrap_or_default());

    let parallel_execution_enabled = record
        .payload
        .get("parallel_execution_enabled")
        .and_then(Value::as_bool);
    summary.latest_parallel_execution_enabled = parallel_execution_enabled;
    if parallel_execution_enabled == Some(true) {
        summary.parallel_execution_enabled_batches =
            summary.parallel_execution_enabled_batches.saturating_add(1);
    }

    let parallel_execution_max_in_flight =
        read_u32_opt(&record.payload, "parallel_execution_max_in_flight");
    summary.latest_parallel_execution_max_in_flight = parallel_execution_max_in_flight;
    if let Some(value) = parallel_execution_max_in_flight {
        summary.parallel_execution_max_in_flight_samples = summary
            .parallel_execution_max_in_flight_samples
            .saturating_add(1);
        summary.parallel_execution_max_in_flight_sum = summary
            .parallel_execution_max_in_flight_sum
            .saturating_add(u64::from(value));
        summary.parallel_execution_max_in_flight_max = Some(
            summary
                .parallel_execution_max_in_flight_max
                .map_or(value, |current| current.max(value)),
        );
    }

    let observed_peak_in_flight = read_u32_opt(&record.payload, "observed_peak_in_flight");
    summary.latest_observed_peak_in_flight = observed_peak_in_flight;
    if let Some(value) = observed_peak_in_flight {
        summary.observed_peak_in_flight_samples =
            summary.observed_peak_in_flight_samples.saturating_add(1);
        summary.observed_peak_in_flight_sum = summary
            .observed_peak_in_flight_sum
            .saturating_add(u64::from(value));
        summary.observed_peak_in_flight_max = Some(
            summary
                .observed_peak_in_flight_max
                .map_or(value, |current| current.max(value)),
        );
    }

    let observed_wall_time_ms = read_u64_opt(&record.payload, "observed_wall_time_ms");
    summary.latest_observed_wall_time_ms = observed_wall_time_ms;
    if let Some(value) = observed_wall_time_ms {
        summary.observed_wall_time_ms_samples =
            summary.observed_wall_time_ms_samples.saturating_add(1);
        summary.observed_wall_time_ms_sum = summary.observed_wall_time_ms_sum.saturating_add(value);
        summary.observed_wall_time_ms_max = Some(
            summary
                .observed_wall_time_ms_max
                .map_or(value, |current| current.max(value)),
        );
    }

    let parallel_safe_intents = read_u32_opt(&record.payload, "parallel_safe_intents");
    summary.latest_parallel_safe_intents = parallel_safe_intents;
    summary.total_parallel_safe_intents_seen = summary
        .total_parallel_safe_intents_seen
        .saturating_add(parallel_safe_intents.map(u64::from).unwrap_or_default());

    let serial_only_intents = read_u32_opt(&record.payload, "serial_only_intents");
    summary.latest_serial_only_intents = serial_only_intents;
    summary.total_serial_only_intents_seen = summary
        .total_serial_only_intents_seen
        .saturating_add(serial_only_intents.map(u64::from).unwrap_or_default());

    let parallel_segments = read_u32_opt(&record.payload, "parallel_segments");
    summary.latest_parallel_segments = parallel_segments;
    summary.total_parallel_segments_seen = summary
        .total_parallel_segments_seen
        .saturating_add(parallel_segments.map(u64::from).unwrap_or_default());

    let sequential_segments = read_u32_opt(&record.payload, "sequential_segments");
    summary.latest_sequential_segments = sequential_segments;
    summary.total_sequential_segments_seen = summary
        .total_sequential_segments_seen
        .saturating_add(sequential_segments.map(u64::from).unwrap_or_default());

    match (
        parallel_segments.unwrap_or_default() > 0,
        sequential_segments.unwrap_or_default() > 0,
    ) {
        (true, false) => {
            summary.parallel_only_batches = summary.parallel_only_batches.saturating_add(1);
        }
        (false, true) => {
            summary.sequential_only_batches = summary.sequential_only_batches.saturating_add(1);
        }
        (true, true) => {
            summary.mixed_execution_batches = summary.mixed_execution_batches.saturating_add(1);
        }
        (false, false) => {
            summary.batches_without_segments = summary.batches_without_segments.saturating_add(1);
        }
    }

    let segments = parse_fast_lane_tool_batch_segments(record.payload.get("segments"));
    for segment in &segments {
        let scheduling_class = segment.scheduling_class.trim();
        if !scheduling_class.is_empty() {
            bump_count(&mut summary.scheduling_class_counts, scheduling_class);
        }
        let execution_mode = segment.execution_mode.trim();
        if !execution_mode.is_empty() {
            bump_count(&mut summary.execution_mode_counts, execution_mode);
        }
        if execution_mode == "parallel" && segment.observed_peak_in_flight == Some(1) {
            summary.degraded_parallel_segments =
                summary.degraded_parallel_segments.saturating_add(1);
        }
    }
    summary.latest_segments = segments;
}

pub(crate) fn summarize_turn_checkpoint_history<'a, I>(
    contents: I,
) -> TurnCheckpointHistoryProjection
where
    I: IntoIterator<Item = &'a str>,
{
    let mut projection = TurnCheckpointHistoryProjection::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };
        if let Some(checkpoint) = fold_turn_checkpoint_event_record(record, &mut projection.summary)
        {
            projection.latest_checkpoint = Some(checkpoint);
        }
    }

    projection.summary.session_state = classify_turn_checkpoint_session_state(
        projection.summary.checkpoint_events,
        projection.summary.latest_stage,
    );
    projection.summary.checkpoint_durable = projection.summary.checkpoint_events > 0;
    projection.summary.reply_durable = projection.summary.latest_persistence_mode.is_some();
    projection.summary.requires_recovery = matches!(
        projection.summary.session_state,
        TurnCheckpointSessionState::PendingFinalization
            | TurnCheckpointSessionState::FinalizationFailed
    );
    projection
}

pub fn summarize_turn_checkpoint_events<'a, I>(contents: I) -> TurnCheckpointEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    summarize_turn_checkpoint_history(contents).summary
}

fn fold_turn_checkpoint_event_record(
    record: ConversationEventRecord,
    summary: &mut TurnCheckpointEventSummary,
) -> Option<Value> {
    if record.event != "turn_checkpoint" {
        return None;
    }

    summary.checkpoint_events = summary.checkpoint_events.saturating_add(1);
    summary.latest_schema_version = record
        .payload
        .get("schema_version")
        .and_then(Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32);

    let stage = record
        .payload
        .get("stage")
        .and_then(Value::as_str)
        .and_then(parse_turn_checkpoint_stage);
    if let Some(raw_stage) = record.payload.get("stage").and_then(Value::as_str) {
        bump_count(&mut summary.stage_counts, raw_stage);
    }
    match stage {
        Some(TurnCheckpointStage::PostPersist) => {
            summary.post_persist_events = summary.post_persist_events.saturating_add(1);
        }
        Some(TurnCheckpointStage::Finalized) => {
            summary.finalized_events = summary.finalized_events.saturating_add(1);
        }
        Some(TurnCheckpointStage::FinalizationFailed) => {
            summary.finalization_failed_events =
                summary.finalization_failed_events.saturating_add(1);
        }
        None => {}
    }
    summary.latest_stage = stage;
    summary.latest_after_turn = record
        .payload
        .get("finalization_progress")
        .and_then(|progress| progress.get("after_turn"))
        .and_then(Value::as_str)
        .and_then(parse_turn_checkpoint_progress_status);
    summary.latest_compaction = record
        .payload
        .get("finalization_progress")
        .and_then(|progress| progress.get("compaction"))
        .and_then(Value::as_str)
        .and_then(parse_turn_checkpoint_progress_status);
    summary.latest_failure_step = record
        .payload
        .get("failure")
        .and_then(|failure| failure.get("step"))
        .and_then(Value::as_str)
        .and_then(parse_turn_checkpoint_failure_step);
    summary.latest_failure_error = record
        .payload
        .get("failure")
        .and_then(|failure| failure.get("error"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    summary.latest_lane = record
        .payload
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("lane"))
        .and_then(|lane| lane.get("lane"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    summary.latest_result_kind = record
        .payload
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("lane"))
        .and_then(|lane| lane.get("result_kind"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    summary.latest_safe_lane_terminal_route = parse_safe_lane_terminal_route_snapshot(
        record
            .payload
            .get("checkpoint")
            .and_then(|checkpoint| checkpoint.get("lane"))
            .and_then(|lane| lane.get("safe_lane_terminal_route")),
    );
    let finalization = record
        .payload
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("finalization"));
    summary.latest_persistence_mode = finalization
        .and_then(|finalization| finalization.get("persistence_mode"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    summary.latest_identity_present = record
        .payload
        .get("checkpoint")
        .map(|checkpoint| checkpoint.get("identity").is_some());
    let legacy_persist_reply = summary.latest_persistence_mode.is_some();
    summary.latest_runs_after_turn = finalization
        .and_then(|finalization| finalization.get("runs_after_turn"))
        .and_then(Value::as_bool)
        .or_else(|| legacy_persist_reply.then_some(true));
    summary.latest_attempts_context_compaction = finalization
        .and_then(|finalization| finalization.get("attempts_context_compaction"))
        .and_then(Value::as_bool)
        .or_else(|| legacy_persist_reply.then_some(true));
    summary.latest_compaction_diagnostics = record
        .payload
        .get("compaction_diagnostics")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());

    record.payload.get("checkpoint").cloned()
}

pub fn build_turn_checkpoint_repair_plan(
    summary: &TurnCheckpointEventSummary,
) -> TurnCheckpointRepairPlan {
    let runs_after_turn = summary.latest_runs_after_turn.unwrap_or(false);
    let attempts_context_compaction = summary.latest_attempts_context_compaction.unwrap_or(false);
    let after_turn_status =
        restore_turn_checkpoint_progress_status(summary.latest_after_turn, runs_after_turn);
    let compaction_status = restore_turn_checkpoint_progress_status(
        summary.latest_compaction,
        attempts_context_compaction,
    );

    if !summary.requires_recovery {
        return TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::None,
            None,
            after_turn_status,
            compaction_status,
        );
    }
    if summary.latest_identity_present != Some(true) {
        return TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::InspectManually,
            Some(TurnCheckpointRepairManualReason::CheckpointIdentityMissing),
            after_turn_status,
            compaction_status,
        );
    }

    let run_after_turn = runs_after_turn
        && matches!(
            after_turn_status,
            TurnCheckpointProgressStatus::Pending
                | TurnCheckpointProgressStatus::Failed
                | TurnCheckpointProgressStatus::FailedOpen
        );
    let run_compaction = attempts_context_compaction
        && match compaction_status {
            TurnCheckpointProgressStatus::Pending
            | TurnCheckpointProgressStatus::Failed
            | TurnCheckpointProgressStatus::FailedOpen => true,
            TurnCheckpointProgressStatus::Skipped => run_after_turn,
            TurnCheckpointProgressStatus::Completed => false,
        };

    match (run_after_turn, run_compaction) {
        (false, false) => TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::InspectManually,
            Some(
                summary
                    .latest_safe_lane_terminal_route
                    .and_then(TurnCheckpointRepairManualReason::from_safe_lane_terminal_route)
                    .unwrap_or(
                        TurnCheckpointRepairManualReason::CheckpointStateRequiresManualInspection,
                    ),
            ),
            after_turn_status,
            compaction_status,
        ),
        (true, false) => TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::RunAfterTurn,
            None,
            after_turn_status,
            compaction_status,
        ),
        (false, true) => TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::RunCompaction,
            None,
            after_turn_status,
            compaction_status,
        ),
        (true, true) => TurnCheckpointRepairPlan::new(
            TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction,
            None,
            after_turn_status,
            compaction_status,
        ),
    }
}

pub fn plan_turn_checkpoint_recovery(
    summary: &TurnCheckpointEventSummary,
) -> TurnCheckpointRecoveryAction {
    build_turn_checkpoint_repair_plan(summary).action()
}

fn restore_turn_checkpoint_progress_status(
    status: Option<TurnCheckpointProgressStatus>,
    expected: bool,
) -> TurnCheckpointProgressStatus {
    match status {
        Some(TurnCheckpointProgressStatus::Pending) => TurnCheckpointProgressStatus::Pending,
        Some(TurnCheckpointProgressStatus::Skipped) => TurnCheckpointProgressStatus::Skipped,
        Some(TurnCheckpointProgressStatus::Completed) => TurnCheckpointProgressStatus::Completed,
        Some(TurnCheckpointProgressStatus::Failed) => TurnCheckpointProgressStatus::Failed,
        Some(TurnCheckpointProgressStatus::FailedOpen) => TurnCheckpointProgressStatus::FailedOpen,
        None if expected => TurnCheckpointProgressStatus::Pending,
        None => TurnCheckpointProgressStatus::Skipped,
    }
}

fn is_backpressure_safe_lane_final_status_payload(payload: &Value) -> bool {
    if payload
        .get("failure_code")
        .and_then(Value::as_str)
        .map(is_safe_lane_backpressure_failure_code)
        .unwrap_or(false)
    {
        return true;
    }
    payload
        .get("route_reason")
        .and_then(Value::as_str)
        .map(is_safe_lane_backpressure_route_reason)
        .unwrap_or(false)
}

fn parse_safe_lane_terminal_route_snapshot(
    value: Option<&Value>,
) -> Option<SafeLaneTerminalRouteSnapshot> {
    let route = value?;
    Some(SafeLaneTerminalRouteSnapshot {
        decision: route
            .get("decision")
            .and_then(Value::as_str)
            .and_then(SafeLaneFailureRouteDecision::parse)?,
        reason: route
            .get("reason")
            .and_then(Value::as_str)
            .and_then(SafeLaneFailureRouteReason::parse)?,
        source: route
            .get("source")
            .and_then(Value::as_str)
            .and_then(SafeLaneFailureRouteSource::parse)?,
    })
}

fn parse_metrics_snapshot(value: Option<&Value>) -> Option<SafeLaneMetricsSnapshot> {
    let metrics = value?;
    let has_any = [
        "rounds_started",
        "rounds_succeeded",
        "rounds_failed",
        "verify_failures",
        "replans_triggered",
        "total_attempts_used",
    ]
    .iter()
    .any(|key| metrics.get(*key).is_some());
    if !has_any {
        return None;
    }

    Some(SafeLaneMetricsSnapshot {
        rounds_started: read_u32(metrics, "rounds_started"),
        rounds_succeeded: read_u32(metrics, "rounds_succeeded"),
        rounds_failed: read_u32(metrics, "rounds_failed"),
        verify_failures: read_u32(metrics, "verify_failures"),
        replans_triggered: read_u32(metrics, "replans_triggered"),
        total_attempts_used: metrics
            .get("total_attempts_used")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    })
}

fn parse_tool_output_snapshot(value: Option<&Value>) -> Option<SafeLaneToolOutputSnapshot> {
    let snapshot = value?;
    let has_any = [
        "output_lines",
        "result_lines",
        "truncated_result_lines",
        "any_truncated",
        "truncation_ratio_milli",
    ]
    .iter()
    .any(|key| snapshot.get(*key).is_some());
    if !has_any {
        return None;
    }

    let output_lines = read_u32(snapshot, "output_lines");
    let result_lines = read_u32(snapshot, "result_lines");
    let truncated_result_lines = read_u32(snapshot, "truncated_result_lines").min(result_lines);
    let any_truncated = snapshot
        .get("any_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(truncated_result_lines > 0);
    let truncation_ratio_milli = snapshot
        .get("truncation_ratio_milli")
        .and_then(Value::as_u64)
        .map(|raw| raw.min(1000) as u32)
        .unwrap_or_else(|| {
            if result_lines == 0 {
                0
            } else {
                ((truncated_result_lines as u64)
                    .saturating_mul(1000)
                    .saturating_div(result_lines as u64))
                .min(1000) as u32
            }
        });

    Some(SafeLaneToolOutputSnapshot {
        output_lines,
        result_lines,
        truncated_result_lines,
        any_truncated,
        truncation_ratio_milli,
    })
}

fn compute_truncation_ratio_milli(truncated_lines: u64, result_lines: u64) -> Option<u32> {
    if result_lines == 0 {
        return None;
    }
    Some(
        truncated_lines
            .saturating_mul(1000)
            .saturating_div(result_lines)
            .min(u32::MAX as u64) as u32,
    )
}

fn parse_health_signal_snapshot(value: Option<&Value>) -> Option<SafeLaneHealthSignalSnapshot> {
    let signal = value?;
    let severity = signal
        .get("severity")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    let flags = signal
        .get("flags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if severity == "unknown" && flags.is_empty() {
        return None;
    }
    Some(SafeLaneHealthSignalSnapshot { severity, flags })
}

fn parse_turn_checkpoint_stage(value: &str) -> Option<TurnCheckpointStage> {
    match value {
        "post_persist" => Some(TurnCheckpointStage::PostPersist),
        "finalized" => Some(TurnCheckpointStage::Finalized),
        "finalization_failed" => Some(TurnCheckpointStage::FinalizationFailed),
        _ => None,
    }
}

fn parse_turn_checkpoint_progress_status(value: &str) -> Option<TurnCheckpointProgressStatus> {
    match value {
        "pending" => Some(TurnCheckpointProgressStatus::Pending),
        "skipped" => Some(TurnCheckpointProgressStatus::Skipped),
        "completed" => Some(TurnCheckpointProgressStatus::Completed),
        "failed" => Some(TurnCheckpointProgressStatus::Failed),
        "failed_open" => Some(TurnCheckpointProgressStatus::FailedOpen),
        _ => None,
    }
}

fn parse_turn_checkpoint_failure_step(value: &str) -> Option<TurnCheckpointFailureStep> {
    match value {
        "after_turn" => Some(TurnCheckpointFailureStep::AfterTurn),
        "compaction" => Some(TurnCheckpointFailureStep::Compaction),
        _ => None,
    }
}

fn classify_turn_checkpoint_session_state(
    checkpoint_events: u32,
    latest_stage: Option<TurnCheckpointStage>,
) -> TurnCheckpointSessionState {
    if checkpoint_events == 0 {
        return TurnCheckpointSessionState::NotDurable;
    }
    match latest_stage {
        Some(TurnCheckpointStage::PostPersist) => TurnCheckpointSessionState::PendingFinalization,
        Some(TurnCheckpointStage::Finalized) => TurnCheckpointSessionState::Finalized,
        Some(TurnCheckpointStage::FinalizationFailed) => {
            TurnCheckpointSessionState::FinalizationFailed
        }
        None => TurnCheckpointSessionState::PendingFinalization,
    }
}

pub(super) fn is_safe_lane_event_name(event_name: &str) -> bool {
    matches!(
        event_name,
        "lane_selected"
            | "plan_round_started"
            | "plan_round_completed"
            | "verify_failed"
            | "verify_policy_adjusted"
            | "replan_triggered"
            | "final_status"
    )
}

fn is_discovery_first_event_name(event_name: &str) -> bool {
    matches!(
        event_name,
        "discovery_first_search_round"
            | "discovery_first_followup_requested"
            | "discovery_first_followup_result"
    )
}

fn read_u32(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|num| num.min(u32::MAX as u64) as u32)
        .unwrap_or_default()
}

fn bump_count(map: &mut BTreeMap<String, u32>, key: &str) {
    let entry = map.entry(key.to_owned()).or_insert(0);
    *entry = entry.saturating_add(1);
}

fn fold_session_governor_summary(
    session_governor: Option<&Value>,
    summary: &mut SafeLaneEventSummary,
) {
    let Some(governor) = session_governor else {
        return;
    };
    summary.session_governor_metrics_snapshots_seen = summary
        .session_governor_metrics_snapshots_seen
        .saturating_add(1);

    if governor
        .get("engaged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_engaged_events =
            summary.session_governor_engaged_events.saturating_add(1);
    }
    if governor
        .get("force_no_replan")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_force_no_replan_events = summary
            .session_governor_force_no_replan_events
            .saturating_add(1);
    }
    if governor
        .get("failed_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_failed_threshold_triggered_events = summary
            .session_governor_failed_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("backpressure_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_backpressure_threshold_triggered_events = summary
            .session_governor_backpressure_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("trend_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_trend_threshold_triggered_events = summary
            .session_governor_trend_threshold_triggered_events
            .saturating_add(1);
    }
    if governor
        .get("recovery_threshold_triggered")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        summary.session_governor_recovery_threshold_triggered_events = summary
            .session_governor_recovery_threshold_triggered_events
            .saturating_add(1);
    }

    summary.session_governor_latest_trend_samples = read_u32_opt(governor, "trend_samples");
    summary.session_governor_latest_trend_min_samples = read_u32_opt(governor, "trend_min_samples");
    summary.session_governor_latest_trend_failure_ewma_milli =
        read_f64_milli_opt(governor, "trend_failure_ewma");
    summary.session_governor_latest_trend_backpressure_ewma_milli =
        read_f64_milli_opt(governor, "trend_backpressure_ewma");
    summary.session_governor_latest_recovery_success_streak =
        read_u32_opt(governor, "recovery_success_streak");
    summary.session_governor_latest_recovery_success_streak_threshold =
        read_u32_opt(governor, "recovery_success_streak_threshold");
}

fn parse_fast_lane_tool_batch_segments(
    value: Option<&Value>,
) -> Vec<FastLaneToolBatchSegmentSnapshot> {
    value
        .and_then(Value::as_array)
        .map(|segments| {
            segments
                .iter()
                .filter_map(|segment| {
                    Some(FastLaneToolBatchSegmentSnapshot {
                        segment_index: read_u32_opt(segment, "segment_index")?,
                        scheduling_class: segment
                            .get("scheduling_class")
                            .and_then(Value::as_str)?
                            .to_owned(),
                        execution_mode: segment
                            .get("execution_mode")
                            .and_then(Value::as_str)?
                            .to_owned(),
                        intent_count: read_u32_opt(segment, "intent_count")?,
                        observed_peak_in_flight: read_u32_opt(segment, "observed_peak_in_flight"),
                        observed_wall_time_ms: read_u64_opt(segment, "observed_wall_time_ms"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_u32_opt(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map(|num| num.min(u32::MAX as u64) as u32)
}

fn read_u64_opt(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn read_f64_milli_opt(value: &Value, key: &str) -> Option<u32> {
    let raw = value.get(key)?.as_f64()?;
    if !raw.is_finite() {
        return None;
    }
    let clamped = raw.clamp(0.0, 1.0);
    let milli = (clamped * 1000.0).round();
    Some(milli.min(u32::MAX as f64) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::SafeLaneFailureCode;
    use crate::conversation::turn_budget::SafeLaneFailureRouteReason;
    use serde_json::json;

    #[test]
    fn parse_conversation_event_rejects_non_event_payloads() {
        assert!(parse_conversation_event("not-json").is_none());
        assert!(parse_conversation_event(r#"{"type":"tool_outcome"}"#).is_none());
    }

    #[test]
    fn summarize_fast_lane_tool_batch_events_tracks_latest_batch_snapshot() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 2,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 2,
                    "observed_peak_in_flight": 2,
                    "observed_wall_time_ms": 24,
                    "parallel_safe_intents": 2,
                    "serial_only_intents": 0,
                    "parallel_segments": 1,
                    "sequential_segments": 0,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 2,
                            "observed_wall_time_ms": 24
                        }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 3,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 3,
                    "observed_peak_in_flight": 2,
                    "observed_wall_time_ms": 41,
                    "parallel_safe_intents": 2,
                    "serial_only_intents": 1,
                    "parallel_segments": 1,
                    "sequential_segments": 1,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 2,
                            "observed_wall_time_ms": 29
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1,
                            "observed_peak_in_flight": 1,
                            "observed_wall_time_ms": 12
                        }
                    ]
                }
            })
            .to_string(),
        ];

        let summary = summarize_fast_lane_tool_batch_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.batch_events, 2);
        assert_eq!(summary.latest_schema_version, Some(2));
        assert_eq!(summary.latest_total_intents, Some(3));
        assert_eq!(summary.latest_parallel_execution_enabled, Some(true));
        assert_eq!(summary.latest_parallel_execution_max_in_flight, Some(3));
        assert_eq!(summary.latest_observed_peak_in_flight, Some(2));
        assert_eq!(summary.latest_observed_wall_time_ms, Some(41));
        assert_eq!(summary.latest_parallel_safe_intents, Some(2));
        assert_eq!(summary.latest_serial_only_intents, Some(1));
        assert_eq!(summary.latest_parallel_segments, Some(1));
        assert_eq!(summary.latest_sequential_segments, Some(1));
        assert_eq!(
            summary.latest_segments,
            vec![
                FastLaneToolBatchSegmentSnapshot {
                    segment_index: 0,
                    scheduling_class: "parallel_safe".to_owned(),
                    execution_mode: "parallel".to_owned(),
                    intent_count: 2,
                    observed_peak_in_flight: Some(2),
                    observed_wall_time_ms: Some(29),
                },
                FastLaneToolBatchSegmentSnapshot {
                    segment_index: 1,
                    scheduling_class: "serial_only".to_owned(),
                    execution_mode: "sequential".to_owned(),
                    intent_count: 1,
                    observed_peak_in_flight: Some(1),
                    observed_wall_time_ms: Some(12),
                },
            ]
        );
    }

    #[test]
    fn summarize_fast_lane_tool_batch_events_tracks_window_aggregates() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 3,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 3,
                    "observed_peak_in_flight": 3,
                    "observed_wall_time_ms": 33,
                    "parallel_safe_intents": 3,
                    "serial_only_intents": 0,
                    "parallel_segments": 2,
                    "sequential_segments": 0,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 2,
                            "observed_wall_time_ms": 18
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 1,
                            "observed_peak_in_flight": 1,
                            "observed_wall_time_ms": 15
                        }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 3,
                    "parallel_execution_enabled": true,
                    "parallel_execution_max_in_flight": 2,
                    "observed_peak_in_flight": 2,
                    "observed_wall_time_ms": 27,
                    "parallel_safe_intents": 2,
                    "serial_only_intents": 1,
                    "parallel_segments": 1,
                    "sequential_segments": 1,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "parallel_safe",
                            "execution_mode": "parallel",
                            "intent_count": 2,
                            "observed_peak_in_flight": 1,
                            "observed_wall_time_ms": 16
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1,
                            "observed_peak_in_flight": 1,
                            "observed_wall_time_ms": 11
                        }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 1,
                    "total_intents": 2,
                    "parallel_execution_enabled": false,
                    "parallel_execution_max_in_flight": 1,
                    "parallel_safe_intents": 0,
                    "serial_only_intents": 2,
                    "parallel_segments": 0,
                    "sequential_segments": 2,
                    "segments": [
                        {
                            "segment_index": 0,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1
                        },
                        {
                            "segment_index": 1,
                            "scheduling_class": "serial_only",
                            "execution_mode": "sequential",
                            "intent_count": 1
                        }
                    ]
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "fast_lane_tool_batch",
                "payload": {
                    "schema_version": 2,
                    "total_intents": 0,
                    "parallel_execution_enabled": false,
                    "parallel_safe_intents": 0,
                    "serial_only_intents": 0,
                    "parallel_segments": 0,
                    "sequential_segments": 0,
                    "segments": []
                }
            })
            .to_string(),
        ];

        let summary = summarize_fast_lane_tool_batch_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.batch_events, 4);
        assert_eq!(summary.parallel_execution_enabled_batches, 2);
        assert_eq!(summary.parallel_only_batches, 1);
        assert_eq!(summary.mixed_execution_batches, 1);
        assert_eq!(summary.sequential_only_batches, 1);
        assert_eq!(summary.batches_without_segments, 1);
        assert_eq!(summary.total_intents_seen, 8);
        assert_eq!(summary.total_parallel_safe_intents_seen, 5);
        assert_eq!(summary.total_serial_only_intents_seen, 3);
        assert_eq!(summary.total_parallel_segments_seen, 3);
        assert_eq!(summary.total_sequential_segments_seen, 3);
        assert_eq!(summary.parallel_execution_max_in_flight_samples, 3);
        assert_eq!(summary.parallel_execution_max_in_flight_sum, 6);
        assert_eq!(summary.parallel_execution_max_in_flight_max, Some(3));
        assert_eq!(summary.observed_peak_in_flight_samples, 2);
        assert_eq!(summary.observed_peak_in_flight_sum, 5);
        assert_eq!(summary.observed_peak_in_flight_max, Some(3));
        assert_eq!(summary.observed_wall_time_ms_samples, 2);
        assert_eq!(summary.observed_wall_time_ms_sum, 60);
        assert_eq!(summary.observed_wall_time_ms_max, Some(33));
        assert_eq!(summary.degraded_parallel_segments, 2);
        assert_eq!(
            summary.scheduling_class_counts,
            BTreeMap::from([
                ("parallel_safe".to_owned(), 3),
                ("serial_only".to_owned(), 3),
            ])
        );
        assert_eq!(
            summary.execution_mode_counts,
            BTreeMap::from([("parallel".to_owned(), 3), ("sequential".to_owned(), 3),])
        );
    }

    #[test]
    fn summarize_fast_lane_tool_batch_events_ignores_malformed_segments() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "fast_lane_tool_batch",
            "payload": {
                "schema_version": 1,
                "total_intents": 2,
                "parallel_execution_enabled": true,
                "parallel_execution_max_in_flight": 2,
                "parallel_safe_intents": 2,
                "serial_only_intents": 0,
                "parallel_segments": 1,
                "sequential_segments": 0,
                "segments": [
                    null,
                    [],
                    {
                        "segment_index": 99
                    },
                    {
                        "segment_index": 1,
                        "scheduling_class": "parallel_safe",
                        "execution_mode": "parallel",
                        "intent_count": 2
                    }
                ]
            }
        })
        .to_string()];

        let summary = summarize_fast_lane_tool_batch_events(payloads.iter().map(String::as_str));

        assert_eq!(
            summary.scheduling_class_counts,
            BTreeMap::from([("parallel_safe".to_owned(), 1)])
        );
        assert_eq!(
            summary.execution_mode_counts,
            BTreeMap::from([("parallel".to_owned(), 1)])
        );
        assert_eq!(
            summary.latest_segments,
            vec![FastLaneToolBatchSegmentSnapshot {
                segment_index: 1,
                scheduling_class: "parallel_safe".to_owned(),
                execution_mode: "parallel".to_owned(),
                intent_count: 2,
                observed_peak_in_flight: None,
                observed_wall_time_ms: None,
            }]
        );
    }

    #[test]
    fn summarize_safe_lane_events_counts_and_final_fields() {
        let payloads = [
            r#"{"type":"conversation_event","event":"lane_selected","payload":{"lane":"safe"}}"#,
            r#"{"type":"conversation_event","event":"plan_round_started","payload":{"round":0}}"#,
            r#"{"type":"conversation_event","event":"plan_round_completed","payload":{"round":0,"status":"failed"}}"#,
            r#"{"type":"conversation_event","event":"verify_policy_adjusted","payload":{"round":0,"min_anchor_matches":1}}"#,
            r#"{"type":"conversation_event","event":"replan_triggered","payload":{"round":0}}"#,
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_verify_failed","route_decision":"terminal"}}"#,
        ];
        let summary = summarize_safe_lane_events(payloads.iter().copied());

        assert_eq!(summary.lane_selected_events, 1);
        assert_eq!(summary.round_started_events, 1);
        assert_eq!(summary.round_completed_failed_events, 1);
        assert_eq!(summary.verify_policy_adjusted_events, 1);
        assert_eq!(summary.replan_triggered_events, 1);
        assert_eq!(summary.final_status_events, 1);
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Failed));
        assert_eq!(
            summary.final_failure_code.as_deref(),
            Some("safe_lane_plan_verify_failed")
        );
        assert_eq!(summary.final_route_decision.as_deref(), Some("terminal"));
        assert_eq!(
            summary.route_decision_counts.get("terminal").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .failure_code_counts
                .get("safe_lane_plan_verify_failed")
                .copied(),
            Some(1)
        );
        assert_eq!(summary.final_status_counts.get("failed").copied(), Some(1));
    }

    #[test]
    fn summarize_safe_lane_events_tracks_latest_metrics_snapshot() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "plan_round_started",
                "payload": {
                    "round": 0,
                    "metrics": {
                        "rounds_started": 1,
                        "rounds_succeeded": 0,
                        "rounds_failed": 0,
                        "verify_failures": 0,
                        "replans_triggered": 0,
                        "total_attempts_used": 0
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "succeeded",
                    "metrics": {
                        "rounds_started": 2,
                        "rounds_succeeded": 1,
                        "rounds_failed": 1,
                        "verify_failures": 0,
                        "replans_triggered": 1,
                        "total_attempts_used": 4
                    }
                }
            })
            .to_string(),
        ];
        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        let metrics = summary.latest_metrics.expect("latest metrics");
        assert_eq!(
            metrics,
            SafeLaneMetricsSnapshot {
                rounds_started: 2,
                rounds_succeeded: 1,
                rounds_failed: 1,
                verify_failures: 0,
                replans_triggered: 1,
                total_attempts_used: 4,
            }
        );
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Succeeded));
        assert_eq!(summary.metrics_snapshots_seen, 2);
        assert_eq!(
            summary.final_status_counts.get("succeeded").copied(),
            Some(1)
        );
    }

    #[test]
    fn summarize_safe_lane_events_accepts_partial_metrics_payload() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "verify_failed",
            "payload": {
                "round": 1,
                "failure_code": "safe_lane_plan_verify_failed",
                "metrics": {
                    "verify_failures": 2
                }
            }
        })
        .to_string()];
        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        let metrics = summary.latest_metrics.expect("latest metrics");
        assert_eq!(metrics.verify_failures, 2);
        assert_eq!(metrics.rounds_started, 0);
        assert_eq!(metrics.total_attempts_used, 0);
        assert_eq!(summary.metrics_snapshots_seen, 1);
    }

    #[test]
    fn summarize_safe_lane_events_handles_sparse_sampled_stream() {
        let payloads = [
            r#"{"type":"conversation_event","event":"lane_selected","payload":{"lane":"safe"}}"#,
            r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_node_retryable_error","route_decision":"terminal","route_reason":"session_governor_no_replan"}}"#,
        ];
        let summary = summarize_safe_lane_events(payloads.iter().copied());
        assert_eq!(summary.lane_selected_events, 1);
        assert_eq!(summary.round_started_events, 0);
        assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Failed));
        assert_eq!(
            summary
                .failure_code_counts
                .get("safe_lane_plan_node_retryable_error")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.route_decision_counts.get("terminal").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .route_reason_counts
                .get("session_governor_no_replan")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.final_route_reason.as_deref(),
            Some("session_governor_no_replan")
        );
    }

    #[test]
    fn safe_lane_event_summary_typed_rollups_track_known_failure_and_route_vocab() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "verify_failed",
                "payload": {
                    "failure_code": "safe_lane_plan_backpressure_guard",
                    "route_reason": "backpressure_attempts_exhausted"
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "failed",
                    "failure_code": "safe_lane_plan_verify_failed_session_governor",
                    "route_reason": "session_governor_no_replan"
                }
            })
            .to_string(),
        ];

        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        assert_eq!(
            summary.typed_final_failure_code(),
            Some(SafeLaneFailureCode::VerifyFailedSessionGovernor)
        );
        assert_eq!(
            summary.typed_final_route_reason(),
            Some(SafeLaneFailureRouteReason::SessionGovernorNoReplan)
        );
        assert_eq!(summary.backpressure_failure_events(), 1);
        assert_eq!(summary.backpressure_route_reason_events(), 1);
        assert!(summary.has_terminal_instability_final_failure());
        assert_eq!(summary.failed_final_status_events(), 1);
    }

    #[test]
    fn safe_lane_event_summary_typed_rollups_ignore_unknown_lookalikes() {
        let mut summary = SafeLaneEventSummary {
            final_status: Some(SafeLaneFinalStatus::Failed),
            final_failure_code: Some("unknown_session_governor_hint".to_owned()),
            final_route_reason: Some("backpressure_noise".to_owned()),
            ..SafeLaneEventSummary::default()
        };
        summary
            .failure_code_counts
            .insert("safe_lane_plan_backpressure_guard".to_owned(), 2);
        summary
            .failure_code_counts
            .insert("unknown_backpressure_hint".to_owned(), 99);
        summary
            .route_reason_counts
            .insert("backpressure_replans_exhausted".to_owned(), 3);
        summary
            .route_reason_counts
            .insert("backpressure_noise".to_owned(), 88);

        assert_eq!(summary.typed_final_failure_code(), None);
        assert_eq!(summary.typed_final_route_reason(), None);
        assert_eq!(summary.backpressure_failure_events(), 2);
        assert_eq!(summary.backpressure_route_reason_events(), 3);
        assert!(!summary.has_terminal_instability_final_failure());
    }

    #[test]
    fn summarize_safe_lane_events_tracks_session_governor_signals() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "lane_selected",
                "payload": {
                    "lane": "safe",
                    "session_governor": {
                        "engaged": true,
                        "force_no_replan": true,
                        "failed_threshold_triggered": true,
                        "backpressure_threshold_triggered": false,
                        "trend_threshold_triggered": true,
                        "recovery_threshold_triggered": false,
                        "trend_samples": 4,
                        "trend_min_samples": 4,
                        "trend_failure_ewma": 0.688,
                        "trend_backpressure_ewma": 0.000,
                        "recovery_success_streak": 0,
                        "recovery_success_streak_threshold": 3
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "plan_round_started",
                "payload": {
                    "round": 0,
                    "session_governor": {
                        "engaged": true,
                        "force_no_replan": true,
                        "failed_threshold_triggered": true,
                        "backpressure_threshold_triggered": false,
                        "trend_threshold_triggered": false,
                        "recovery_threshold_triggered": true,
                        "trend_samples": 5,
                        "trend_min_samples": 4,
                        "trend_failure_ewma": 0.250,
                        "trend_backpressure_ewma": 0.063,
                        "recovery_success_streak": 4,
                        "recovery_success_streak_threshold": 3
                    }
                }
            })
            .to_string(),
        ];

        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.session_governor_engaged_events, 2);
        assert_eq!(summary.session_governor_force_no_replan_events, 2);
        assert_eq!(
            summary.session_governor_failed_threshold_triggered_events,
            2
        );
        assert_eq!(
            summary.session_governor_backpressure_threshold_triggered_events,
            0
        );
        assert_eq!(summary.session_governor_trend_threshold_triggered_events, 1);
        assert_eq!(
            summary.session_governor_recovery_threshold_triggered_events,
            1
        );
        assert_eq!(summary.session_governor_metrics_snapshots_seen, 2);
        assert_eq!(summary.session_governor_latest_trend_samples, Some(5));
        assert_eq!(summary.session_governor_latest_trend_min_samples, Some(4));
        assert_eq!(
            summary.session_governor_latest_trend_failure_ewma_milli,
            Some(250)
        );
        assert_eq!(
            summary.session_governor_latest_trend_backpressure_ewma_milli,
            Some(63)
        );
        assert_eq!(
            summary.session_governor_latest_recovery_success_streak,
            Some(4)
        );
        assert_eq!(
            summary.session_governor_latest_recovery_success_streak_threshold,
            Some(3)
        );
    }

    #[test]
    fn summarize_safe_lane_events_tracks_tool_output_snapshot_rollups() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "plan_round_completed",
                "payload": {
                    "round": 0,
                    "status": "succeeded",
                    "tool_output_stats": {
                        "output_lines": 2,
                        "result_lines": 2,
                        "truncated_result_lines": 1,
                        "any_truncated": true,
                        "truncation_ratio_milli": 500
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "succeeded",
                    "tool_output_stats": {
                        "output_lines": 1,
                        "result_lines": 1,
                        "truncated_result_lines": 0,
                        "any_truncated": false,
                        "truncation_ratio_milli": 0
                    }
                }
            })
            .to_string(),
        ];
        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.tool_output_snapshots_seen, 2);
        assert_eq!(summary.tool_output_truncated_events, 1);
        assert_eq!(summary.tool_output_result_lines_total, 3);
        assert_eq!(summary.tool_output_truncated_result_lines_total, 1);
        assert_eq!(
            summary.tool_output_aggregate_truncation_ratio_milli,
            Some(333)
        );
        assert_eq!(summary.tool_output_truncation_verify_failed_events, 0);
        assert_eq!(summary.tool_output_truncation_replan_events, 0);
        assert_eq!(summary.tool_output_truncation_final_failure_events, 0);
        assert_eq!(
            summary.latest_tool_output,
            Some(SafeLaneToolOutputSnapshot {
                output_lines: 1,
                result_lines: 1,
                truncated_result_lines: 0,
                any_truncated: false,
                truncation_ratio_milli: 0,
            })
        );
    }

    #[test]
    fn summarize_safe_lane_events_tracks_truncation_failure_correlation_counters() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "verify_failed",
                "payload": {
                    "failure_code": "safe_lane_plan_verify_failed",
                    "tool_output_stats": {
                        "output_lines": 2,
                        "result_lines": 2,
                        "truncated_result_lines": 1,
                        "any_truncated": true,
                        "truncation_ratio_milli": 500
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "replan_triggered",
                "payload": {
                    "tool_output_stats": {
                        "output_lines": 1,
                        "result_lines": 1,
                        "truncated_result_lines": 1,
                        "any_truncated": true,
                        "truncation_ratio_milli": 1000
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "failed",
                    "failure_code": "safe_lane_plan_verify_failed",
                    "tool_output_stats": {
                        "output_lines": 1,
                        "result_lines": 1,
                        "truncated_result_lines": 1,
                        "any_truncated": true,
                        "truncation_ratio_milli": 1000
                    }
                }
            })
            .to_string(),
        ];

        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.tool_output_snapshots_seen, 3);
        assert_eq!(summary.tool_output_truncated_events, 3);
        assert_eq!(summary.tool_output_result_lines_total, 4);
        assert_eq!(summary.tool_output_truncated_result_lines_total, 3);
        assert_eq!(
            summary.tool_output_aggregate_truncation_ratio_milli,
            Some(750)
        );
        assert_eq!(summary.tool_output_truncation_verify_failed_events, 1);
        assert_eq!(summary.tool_output_truncation_replan_events, 1);
        assert_eq!(summary.tool_output_truncation_final_failure_events, 1);
    }

    #[test]
    fn summarize_safe_lane_events_tracks_health_signal_rollups() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "plan_round_completed",
                "payload": {
                    "round": 0,
                    "status": "failed",
                    "health_signal": {
                        "severity": "warn",
                        "flags": ["truncation_pressure(0.300)"]
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "failed",
                    "health_signal": {
                        "severity": "critical",
                        "flags": ["terminal_instability"]
                    }
                }
            })
            .to_string(),
        ];

        let summary = summarize_safe_lane_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.health_signal_snapshots_seen, 2);
        assert_eq!(summary.health_signal_warn_events, 1);
        assert_eq!(summary.health_signal_critical_events, 1);
        assert_eq!(
            summary.latest_health_signal,
            Some(SafeLaneHealthSignalSnapshot {
                severity: "critical".to_owned(),
                flags: vec!["terminal_instability".to_owned()],
            })
        );
    }

    #[test]
    fn summarize_safe_lane_history_tracks_governor_samples_with_summary() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "lane_selected",
                "payload": {
                    "lane": "safe"
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "failed",
                    "failure_code": "safe_lane_plan_backpressure_guard",
                    "route_reason": "backpressure_attempts_exhausted",
                    "route_decision": "terminal"
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "final_status",
                "payload": {
                    "status": "succeeded"
                }
            })
            .to_string(),
        ];

        let projection = summarize_safe_lane_history(payloads.iter().map(String::as_str));

        assert_eq!(projection.summary.lane_selected_events, 1);
        assert_eq!(projection.summary.final_status_events, 2);
        assert_eq!(projection.summary.failed_final_status_events(), 1);
        assert_eq!(
            projection.summary.final_status,
            Some(SafeLaneFinalStatus::Succeeded)
        );
        assert_eq!(projection.summary.final_failure_code, None);
        assert_eq!(projection.final_status_failed_samples, vec![true, false]);
        assert_eq!(projection.backpressure_failure_samples, vec![true, false]);
    }

    #[test]
    fn summarize_turn_checkpoint_events_tracks_latest_finalized_state() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "post_persist",
                    "checkpoint": {
                        "identity": {
                            "user_input_sha256": "u1",
                            "assistant_reply_sha256": "a1",
                            "user_input_chars": 5,
                            "assistant_reply_chars": 6
                        },
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_error"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "pending",
                        "compaction": "pending"
                    },
                    "failure": null
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "finalized",
                    "checkpoint": {
                        "identity": {
                            "user_input_sha256": "u2",
                            "assistant_reply_sha256": "a2",
                            "user_input_chars": 7,
                            "assistant_reply_chars": 8
                        },
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_error",
                            "safe_lane_terminal_route": {
                                "decision": "terminal",
                                "reason": "session_governor_no_replan",
                                "source": "session_governor"
                            }
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "completed",
                        "compaction": "failed_open"
                    },
                    "failure": null
                }
            })
            .to_string(),
        ];

        let summary = summarize_turn_checkpoint_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.checkpoint_events, 2);
        assert_eq!(summary.post_persist_events, 1);
        assert_eq!(summary.finalized_events, 1);
        assert_eq!(summary.finalization_failed_events, 0);
        assert_eq!(summary.latest_schema_version, Some(1));
        assert_eq!(summary.latest_stage, Some(TurnCheckpointStage::Finalized));
        assert_eq!(
            summary.latest_after_turn,
            Some(TurnCheckpointProgressStatus::Completed)
        );
        assert_eq!(
            summary.latest_compaction,
            Some(TurnCheckpointProgressStatus::FailedOpen)
        );
        assert_eq!(summary.latest_lane.as_deref(), Some("safe"));
        assert_eq!(summary.latest_result_kind.as_deref(), Some("tool_error"));
        assert_eq!(summary.latest_persistence_mode.as_deref(), Some("success"));
        assert_eq!(
            summary.latest_safe_lane_terminal_route,
            Some(SafeLaneTerminalRouteSnapshot {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            })
        );
        assert_eq!(summary.latest_identity_present, Some(true));
        assert_eq!(summary.latest_runs_after_turn, Some(true));
        assert_eq!(summary.latest_attempts_context_compaction, Some(true));
        assert_eq!(summary.session_state, TurnCheckpointSessionState::Finalized);
        assert_eq!(
            plan_turn_checkpoint_recovery(&summary),
            TurnCheckpointRecoveryAction::None
        );
        assert!(summary.checkpoint_durable);
        assert!(summary.reply_durable);
        assert!(!summary.requires_recovery);
        assert_eq!(summary.stage_counts.get("post_persist").copied(), Some(1));
        assert_eq!(summary.stage_counts.get("finalized").copied(), Some(1));
    }

    #[test]
    fn summarize_turn_checkpoint_events_flags_failed_finalization_for_recovery() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "inline_provider_error"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failure"
                }
            }
        })
        .to_string()];

        let summary = summarize_turn_checkpoint_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.checkpoint_events, 1);
        assert_eq!(
            summary.latest_stage,
            Some(TurnCheckpointStage::FinalizationFailed)
        );
        assert_eq!(
            summary.latest_after_turn,
            Some(TurnCheckpointProgressStatus::Completed)
        );
        assert_eq!(
            summary.latest_compaction,
            Some(TurnCheckpointProgressStatus::Failed)
        );
        assert_eq!(
            summary.latest_failure_step,
            Some(TurnCheckpointFailureStep::Compaction)
        );
        assert_eq!(
            summary.latest_failure_error.as_deref(),
            Some("compact failure")
        );
        assert_eq!(
            summary.latest_persistence_mode.as_deref(),
            Some("inline_provider_error")
        );
        assert_eq!(summary.latest_identity_present, Some(false));
        assert_eq!(summary.latest_runs_after_turn, Some(true));
        assert_eq!(summary.latest_attempts_context_compaction, Some(true));
        assert_eq!(
            summary.session_state,
            TurnCheckpointSessionState::FinalizationFailed
        );
        assert_eq!(
            plan_turn_checkpoint_recovery(&summary),
            TurnCheckpointRecoveryAction::InspectManually
        );
        assert!(summary.checkpoint_durable);
        assert!(summary.reply_durable);
        assert!(summary.requires_recovery);
    }

    #[test]
    fn summarize_turn_checkpoint_events_keeps_return_error_finalized_without_reply_durability() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalized",
                "checkpoint": {
                    "request": {
                        "kind": "return_error"
                    },
                    "finalization": {
                        "kind": "return_error"
                    }
                },
                "finalization_progress": {
                    "after_turn": "skipped",
                    "compaction": "skipped"
                },
                "failure": null
            }
        })
        .to_string()];

        let summary = summarize_turn_checkpoint_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.checkpoint_events, 1);
        assert_eq!(summary.latest_stage, Some(TurnCheckpointStage::Finalized));
        assert_eq!(summary.session_state, TurnCheckpointSessionState::Finalized);
        assert_eq!(
            plan_turn_checkpoint_recovery(&summary),
            TurnCheckpointRecoveryAction::None
        );
        assert!(summary.checkpoint_durable);
        assert!(!summary.reply_durable);
        assert!(!summary.requires_recovery);
        assert_eq!(summary.latest_persistence_mode, None);
        assert_eq!(summary.latest_identity_present, Some(false));
    }

    #[test]
    fn summarize_turn_checkpoint_history_tracks_latest_checkpoint_payload_with_summary() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "post_persist",
                    "checkpoint": {
                        "identity": {
                            "user_input_sha256": "u1",
                            "assistant_reply_sha256": "a1",
                            "user_input_chars": 5,
                            "assistant_reply_chars": 6
                        },
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_call"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "pending",
                        "compaction": "pending"
                    },
                    "failure": null
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "finalization_failed",
                    "checkpoint": {
                        "identity": {
                            "user_input_sha256": "u2",
                            "assistant_reply_sha256": "a2",
                            "user_input_chars": 7,
                            "assistant_reply_chars": 8
                        },
                        "lane": {
                            "lane": "fast",
                            "result_kind": "final_text"
                        },
                        "finalization": {
                            "persistence_mode": "success",
                            "runs_after_turn": true,
                            "attempts_context_compaction": true
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "completed",
                        "compaction": "failed"
                    },
                    "failure": {
                        "step": "compaction",
                        "error": "compact failure"
                    }
                }
            })
            .to_string(),
        ];

        let projection = summarize_turn_checkpoint_history(payloads.iter().map(String::as_str));

        assert_eq!(projection.summary.checkpoint_events, 2);
        assert_eq!(
            projection.summary.latest_stage,
            Some(TurnCheckpointStage::FinalizationFailed)
        );
        assert_eq!(
            projection.summary.latest_after_turn,
            Some(TurnCheckpointProgressStatus::Completed)
        );
        assert_eq!(
            projection.summary.latest_compaction,
            Some(TurnCheckpointProgressStatus::Failed)
        );
        assert_eq!(
            projection.summary.latest_failure_step,
            Some(TurnCheckpointFailureStep::Compaction)
        );
        assert_eq!(projection.summary.latest_lane.as_deref(), Some("fast"));
        assert_eq!(
            projection.summary.latest_result_kind.as_deref(),
            Some("final_text")
        );
        assert!(projection.summary.requires_recovery);
        assert!(projection.summary.checkpoint_durable);
        assert_eq!(
            projection
                .latest_checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.get("lane"))
                .and_then(|lane| lane.get("lane"))
                .and_then(Value::as_str),
            Some("fast")
        );
        assert_eq!(
            projection
                .latest_checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.get("finalization"))
                .and_then(|finalization| finalization.get("attempts_context_compaction"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn plan_turn_checkpoint_recovery_restarts_after_turn_and_compaction_when_needed() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Failed),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_failure_step: Some(TurnCheckpointFailureStep::AfterTurn),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::FinalizationFailed,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        assert_eq!(
            plan_turn_checkpoint_recovery(&summary),
            TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction
        );
    }

    #[test]
    fn plan_turn_checkpoint_recovery_requires_manual_inspection_without_identity() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
            latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
            latest_identity_present: Some(false),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        assert_eq!(
            plan_turn_checkpoint_recovery(&summary),
            TurnCheckpointRecoveryAction::InspectManually
        );
    }

    #[test]
    fn build_turn_checkpoint_repair_plan_marks_missing_identity_as_manual_reason() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
            latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
            latest_identity_present: Some(false),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let plan = build_turn_checkpoint_repair_plan(&summary);

        assert_eq!(plan.action(), TurnCheckpointRecoveryAction::InspectManually);
        assert_eq!(
            plan.manual_reason(),
            Some(TurnCheckpointRepairManualReason::CheckpointIdentityMissing)
        );
        assert!(!plan.should_run_after_turn());
        assert!(!plan.should_run_compaction());
        assert_eq!(
            plan.after_turn_status(),
            TurnCheckpointProgressStatus::Pending
        );
        assert_eq!(
            plan.compaction_status(),
            TurnCheckpointProgressStatus::Pending
        );
    }

    #[test]
    fn build_turn_checkpoint_repair_plan_restores_tail_progress_and_remaining_steps() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
            latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
            latest_failure_step: Some(TurnCheckpointFailureStep::Compaction),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(true),
            latest_attempts_context_compaction: Some(true),
            session_state: TurnCheckpointSessionState::FinalizationFailed,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let plan = build_turn_checkpoint_repair_plan(&summary);

        assert_eq!(plan.action(), TurnCheckpointRecoveryAction::RunCompaction);
        assert_eq!(plan.manual_reason(), None);
        assert!(!plan.should_run_after_turn());
        assert!(plan.should_run_compaction());
        assert_eq!(
            plan.after_turn_status(),
            TurnCheckpointProgressStatus::Completed
        );
        assert_eq!(
            plan.compaction_status(),
            TurnCheckpointProgressStatus::Failed
        );
    }

    #[test]
    fn build_turn_checkpoint_repair_plan_preserves_safe_lane_override_route_in_manual_reason() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_safe_lane_terminal_route: Some(SafeLaneTerminalRouteSnapshot {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
                source: SafeLaneFailureRouteSource::BackpressureGuard,
            }),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let plan = build_turn_checkpoint_repair_plan(&summary);

        assert_eq!(plan.action(), TurnCheckpointRecoveryAction::InspectManually);
        assert_eq!(
            plan.manual_reason()
                .map(TurnCheckpointRepairManualReason::as_str),
            Some("safe_lane_backpressure_terminal_requires_manual_inspection")
        );
        assert!(!plan.should_run_after_turn());
        assert!(!plan.should_run_compaction());
    }

    #[test]
    fn build_turn_checkpoint_repair_plan_keeps_replan_routes_out_of_manual_override_reason() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_safe_lane_terminal_route: Some(SafeLaneTerminalRouteSnapshot {
                decision: SafeLaneFailureRouteDecision::Replan,
                reason: SafeLaneFailureRouteReason::RetryableFailure,
                source: SafeLaneFailureRouteSource::BackpressureGuard,
            }),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let plan = build_turn_checkpoint_repair_plan(&summary);

        assert_eq!(plan.action(), TurnCheckpointRecoveryAction::InspectManually);
        assert_eq!(
            plan.manual_reason()
                .map(TurnCheckpointRepairManualReason::as_str),
            Some("checkpoint_state_requires_manual_inspection")
        );
    }

    #[test]
    fn build_turn_checkpoint_repair_plan_ignores_inconsistent_override_route_pairs() {
        let summary = TurnCheckpointEventSummary {
            checkpoint_events: 1,
            latest_stage: Some(TurnCheckpointStage::PostPersist),
            latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
            latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
            latest_safe_lane_terminal_route: Some(SafeLaneTerminalRouteSnapshot {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::RetryableFailure,
                source: SafeLaneFailureRouteSource::BackpressureGuard,
            }),
            latest_identity_present: Some(true),
            latest_runs_after_turn: Some(false),
            latest_attempts_context_compaction: Some(false),
            session_state: TurnCheckpointSessionState::PendingFinalization,
            checkpoint_durable: true,
            requires_recovery: true,
            reply_durable: true,
            ..TurnCheckpointEventSummary::default()
        };

        let plan = build_turn_checkpoint_repair_plan(&summary);

        assert_eq!(plan.action(), TurnCheckpointRecoveryAction::InspectManually);
        assert_eq!(
            plan.manual_reason()
                .map(TurnCheckpointRepairManualReason::as_str),
            Some("checkpoint_state_requires_manual_inspection")
        );
    }

    #[test]
    fn turn_checkpoint_event_summary_route_labels_default_to_dash_without_snapshot() {
        let summary = TurnCheckpointEventSummary::default();

        assert_eq!(
            summary.latest_safe_lane_route_labels_or_default(),
            ("-", "-", "-")
        );
    }

    #[test]
    fn turn_checkpoint_event_summary_route_labels_project_typed_snapshot() {
        let summary = TurnCheckpointEventSummary {
            latest_safe_lane_terminal_route: Some(SafeLaneTerminalRouteSnapshot {
                decision: SafeLaneFailureRouteDecision::Terminal,
                reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                source: SafeLaneFailureRouteSource::SessionGovernor,
            }),
            ..TurnCheckpointEventSummary::default()
        };

        assert_eq!(
            summary.latest_safe_lane_route_labels_or_default(),
            ("terminal", "session_governor_no_replan", "session_governor")
        );
    }

    #[test]
    fn summarize_discovery_first_events_counts_followup_and_tokens() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": "discovery_first_search_round",
                "payload": {
                    "provider_round": 0,
                    "search_tool_calls": 1,
                    "raw_tool_output_requested": false,
                    "initial_estimated_tokens": 12
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "discovery_first_followup_requested",
                "payload": {
                    "provider_round": 1,
                    "raw_tool_output_requested": true,
                    "initial_estimated_tokens": 12,
                    "followup_estimated_tokens": 21,
                    "followup_added_estimated_tokens": 9
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": "discovery_first_followup_result",
                "payload": {
                    "provider_round": 1,
                    "outcome": "tool.invoke",
                    "followup_tool_name": "tool.invoke",
                    "followup_target_tool_id": "file.read",
                    "resolved_to_tool_invoke": true,
                    "raw_tool_output_requested": true
                }
            })
            .to_string(),
        ];

        let summary = summarize_discovery_first_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.search_round_events, 1);
        assert_eq!(summary.followup_requested_events, 1);
        assert_eq!(summary.followup_result_events, 1);
        assert_eq!(summary.raw_output_followup_events, 1);
        assert_eq!(summary.search_to_invoke_hits, 1);
        assert_eq!(summary.aggregate_added_estimated_tokens, 9);
        assert_eq!(summary.average_added_estimated_tokens, Some(9));
        assert_eq!(
            summary.latest_followup_outcome.as_deref(),
            Some("tool.invoke")
        );
        assert_eq!(
            summary.latest_followup_tool_name.as_deref(),
            Some("tool.invoke")
        );
        assert_eq!(
            summary.latest_followup_target_tool_id.as_deref(),
            Some("file.read")
        );
        assert_eq!(summary.latest_initial_estimated_tokens, Some(12));
        assert_eq!(summary.latest_followup_estimated_tokens, Some(21));
        assert_eq!(summary.latest_added_estimated_tokens, Some(9));
        assert_eq!(summary.outcome_counts.get("tool.invoke").copied(), Some(1));
    }

    #[test]
    fn summarize_discovery_first_events_ignores_lookalikes_and_tracks_latest_request_snapshot() {
        let payloads = [
            r#"{"type":"conversation_event","event":"discovery_first_search_round","payload":{"provider_round":0,"search_tool_calls":1}}"#,
            r#"{"type":"conversation_event","event":"discovery_first_followup_requested","payload":{"provider_round":1,"initial_estimated_tokens":20,"followup_estimated_tokens":32,"followup_added_estimated_tokens":12}}"#,
            r#"{"type":"conversation_event","event":"discovery_first_followup_result","payload":{"provider_round":1,"outcome":"final_reply","resolved_to_tool_invoke":false}}"#,
            r#"{"type":"conversation_event","event":"discovery_first_followup_noise","payload":{"outcome":"tool.invoke","resolved_to_tool_invoke":true,"followup_added_estimated_tokens":999}}"#,
            r#"{"type":"tool_outcome","event":"discovery_first_followup_result","payload":{"outcome":"tool.invoke"}}"#,
        ];

        let summary = summarize_discovery_first_events(payloads.iter().copied());
        assert_eq!(summary.search_round_events, 1);
        assert_eq!(summary.followup_requested_events, 1);
        assert_eq!(summary.followup_result_events, 1);
        assert_eq!(summary.raw_output_followup_events, 0);
        assert_eq!(summary.search_to_invoke_hits, 0);
        assert_eq!(summary.aggregate_added_estimated_tokens, 12);
        assert_eq!(summary.average_added_estimated_tokens, Some(12));
        assert_eq!(
            summary.latest_followup_outcome.as_deref(),
            Some("final_reply")
        );
        assert_eq!(summary.latest_followup_tool_name, None);
        assert_eq!(summary.latest_followup_target_tool_id, None);
        assert_eq!(summary.latest_initial_estimated_tokens, Some(20));
        assert_eq!(summary.latest_followup_estimated_tokens, Some(32));
        assert_eq!(summary.latest_added_estimated_tokens, Some(12));
        assert_eq!(summary.outcome_counts.get("final_reply").copied(), Some(1));
        assert_eq!(summary.outcome_counts.get("tool.invoke").copied(), None);
    }

    #[test]
    fn summarize_prompt_frame_events_tracks_hash_changes_and_advisory_profile() {
        let payloads = [
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":1,"phase":"initial","prompt_frame":{"schema_version":1,"total_estimated_tokens":42,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":0,"turn_ephemeral_estimated_tokens":0,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-a","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":null}}}"#,
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":2,"phase":"followup","prompt_frame":{"schema_version":1,"total_estimated_tokens":55,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":2,"turn_ephemeral_estimated_tokens":11,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-b","advisory_profile_hash":"profile-b","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":"tail-b"}}}"#,
        ];

        let summary = summarize_prompt_frame_events(payloads.iter().copied());

        assert_eq!(summary.snapshot_events, 2);
        assert_eq!(summary.initial_snapshot_events, 1);
        assert_eq!(summary.followup_snapshot_events, 1);
        assert_eq!(summary.stable_prefix_hash_change_events, 0);
        assert_eq!(summary.cached_prefix_hash_change_events, 1);
        assert_eq!(summary.advisory_profile_hash_change_events, 1);
        assert_eq!(summary.turn_ephemeral_hash_change_events, 0);
        assert_eq!(summary.latest_phase.as_deref(), Some("followup"));
        assert_eq!(summary.latest_total_estimated_tokens, Some(55));
        assert_eq!(summary.latest_advisory_profile_segment_count, Some(1));
        assert_eq!(summary.latest_advisory_profile_estimated_tokens, Some(6));
        assert_eq!(
            summary.latest_advisory_profile_hash.as_deref(),
            Some("profile-b")
        );
        assert_eq!(
            summary.latest_stable_prefix_hash.as_deref(),
            Some("prefix-a")
        );
        assert_eq!(
            summary.latest_cached_prefix_hash.as_deref(),
            Some("cached-b")
        );
        assert_eq!(summary.latest_turn_ephemeral_segment_count, Some(2));
        assert_eq!(
            summary.latest_turn_ephemeral_hash.as_deref(),
            Some("tail-b")
        );
    }

    #[test]
    fn summarize_prompt_frame_events_preserves_previous_hash_across_missing_tail_hash() {
        let payloads = [
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":1,"phase":"initial","prompt_frame":{"schema_version":1,"total_estimated_tokens":42,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":1,"turn_ephemeral_estimated_tokens":4,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-a","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":"tail-a"}}}"#,
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":2,"phase":"followup","prompt_frame":{"schema_version":1,"total_estimated_tokens":55,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":0,"turn_ephemeral_estimated_tokens":0,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-a","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":null}}}"#,
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":3,"phase":"followup","prompt_frame":{"schema_version":1,"total_estimated_tokens":60,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":1,"turn_ephemeral_estimated_tokens":5,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-a","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":"tail-b"}}}"#,
        ];

        let summary = summarize_prompt_frame_events(payloads.iter().copied());

        assert_eq!(summary.turn_ephemeral_hash_change_events, 1);
        assert_eq!(
            summary.latest_turn_ephemeral_hash.as_deref(),
            Some("tail-b")
        );
    }

    #[test]
    fn summarize_prompt_frame_events_counts_session_latched_drift_as_prefix_drift() {
        let payloads = [
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":1,"phase":"initial","prompt_frame":{"schema_version":1,"total_estimated_tokens":42,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":0,"turn_ephemeral_estimated_tokens":0,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-a","stable_prefix_hash_sha256":"prefix-a","cached_prefix_sha256":"cached-a","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":null}}}"#,
            r#"{"type":"conversation_event","event":"provider_prompt_frame_snapshot","payload":{"provider_round":2,"phase":"followup","prompt_frame":{"schema_version":1,"total_estimated_tokens":55,"stable_runtime_segment_count":1,"stable_runtime_estimated_tokens":12,"session_latched_segment_count":1,"session_latched_estimated_tokens":8,"advisory_profile_segment_count":1,"advisory_profile_estimated_tokens":6,"session_local_recall_segment_count":1,"session_local_recall_estimated_tokens":5,"recent_window_segment_count":1,"recent_window_estimated_tokens":7,"turn_ephemeral_segment_count":2,"turn_ephemeral_estimated_tokens":11,"stable_runtime_hash":"stable-a","session_latched_hash":"latched-b","stable_prefix_hash_sha256":"prefix-b","cached_prefix_sha256":"cached-b","advisory_profile_hash":"profile-a","session_local_recall_hash":"recall-a","recent_window_hash":"window-a","turn_ephemeral_hash":"tail-b"}}}"#,
        ];

        let summary = summarize_prompt_frame_events(payloads.iter().copied());

        assert_eq!(summary.snapshot_events, 2);
        assert_eq!(summary.stable_runtime_hash_change_events, 0);
        assert_eq!(summary.session_latched_hash_change_events, 1);
        assert_eq!(summary.stable_prefix_hash_change_events, 1);
        assert_eq!(summary.cached_prefix_hash_change_events, 1);
        assert_eq!(summary.advisory_profile_hash_change_events, 0);
        assert_eq!(summary.latest_phase.as_deref(), Some("followup"));
        assert_eq!(
            summary.latest_session_latched_hash.as_deref(),
            Some("latched-b")
        );
        assert_eq!(
            summary.latest_stable_prefix_hash.as_deref(),
            Some("prefix-b")
        );
        assert_eq!(
            summary.latest_cached_prefix_hash.as_deref(),
            Some("cached-b")
        );
    }

    #[test]
    fn summarize_discovery_first_events_preserves_initial_tokens_when_call_count_is_missing() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "discovery_first_search_round",
            "payload": {
                "initial_estimated_tokens": 21
            }
        })
        .to_string()];

        let summary = summarize_discovery_first_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.search_round_events, 0);
        assert_eq!(summary.latest_initial_estimated_tokens, Some(21));
    }

    #[test]
    fn summarize_discovery_first_events_preserves_followup_flags_when_outcome_is_missing() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": "discovery_first_followup_result",
            "payload": {
                "raw_tool_output_requested": true,
                "resolved_to_tool_invoke": true,
                "followup_tool_name": "tool.invoke",
                "followup_target_tool_id": "file.read"
            }
        })
        .to_string()];

        let summary = summarize_discovery_first_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.followup_result_events, 0);
        assert_eq!(summary.raw_output_followup_events, 1);
        assert_eq!(summary.search_to_invoke_hits, 1);
        assert_eq!(
            summary.latest_followup_tool_name.as_deref(),
            Some("tool.invoke")
        );
        assert_eq!(
            summary.latest_followup_target_tool_id.as_deref(),
            Some("file.read")
        );
        assert_eq!(summary.latest_followup_outcome, None);
        assert!(summary.outcome_counts.is_empty());
    }
}
