use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ConversationEventRecord, bump_count, parse_conversation_event};
use crate::conversation::ContextCompactionDiagnostics;
use crate::conversation::safe_lane_failure::{
    SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource, SafeLaneTerminalRouteSnapshot,
};
use crate::conversation::turn_budget::SafeLaneFailureRouteReason;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
