use serde_json::json;

use crate::mvp;

pub(crate) const RUNTIME_SNAPSHOT_COMPACTION_HYGIENE_EVENT_LIMIT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeSnapshotCheckpointRepairManualReason {
    CheckpointIdentityMissing,
    SafeLaneBackpressureTerminalRequiresManualInspection,
    SafeLaneSessionGovernorTerminalRequiresManualInspection,
    CheckpointStateRequiresManualInspection,
}

impl RuntimeSnapshotCheckpointRepairManualReason {
    pub(crate) const fn as_str(self) -> &'static str {
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
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PrimaryLineageCheckpointProjection {
    pub(crate) latest_compaction_status: Option<mvp::conversation::TurnCheckpointProgressStatus>,
    pub(crate) event_count: usize,
    pub(crate) failure_streak: usize,
    pub(crate) repair_action: Option<mvp::conversation::TurnCheckpointRecoveryAction>,
    pub(crate) repair_manual_reason: Option<RuntimeSnapshotCheckpointRepairManualReason>,
}

pub(crate) fn collect_primary_lineage_checkpoint_projection(
    repository: &mvp::session::repository::SessionRepository,
    session_ids: &[String],
) -> PrimaryLineageCheckpointProjection {
    let mut events = Vec::new();

    for session_id in session_ids {
        if session_id.is_empty() {
            continue;
        }
        let Ok(session_events) = repository.list_recent_events(
            session_id.as_str(),
            RUNTIME_SNAPSHOT_COMPACTION_HYGIENE_EVENT_LIMIT,
        ) else {
            continue;
        };
        events.extend(
            session_events
                .into_iter()
                .filter(|event| event.event_kind == "turn_checkpoint"),
        );
    }

    project_primary_lineage_checkpoint_events(&events)
}

pub(crate) fn project_primary_lineage_checkpoint_events(
    events: &[mvp::session::repository::SessionEventRecord],
) -> PrimaryLineageCheckpointProjection {
    if events.is_empty() {
        return PrimaryLineageCheckpointProjection::default();
    }

    let mut events = events.to_vec();
    events.sort_by(|left, right| right.ts.cmp(&left.ts).then_with(|| right.id.cmp(&left.id)));

    let statuses = extract_turn_checkpoint_compaction_statuses(&events);
    let contents = events
        .iter()
        .rev()
        .map(|event| {
            json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": event.payload_json.clone(),
            })
            .to_string()
        })
        .collect::<Vec<_>>();
    let summary =
        mvp::conversation::summarize_turn_checkpoint_events(contents.iter().map(String::as_str));
    let repair_plan = mvp::conversation::build_turn_checkpoint_repair_plan(&summary);

    PrimaryLineageCheckpointProjection {
        latest_compaction_status: summary
            .latest_compaction
            .or_else(|| statuses.first().copied()),
        event_count: events.len(),
        failure_streak: statuses
            .iter()
            .take_while(|status| is_failure_status(**status))
            .count(),
        repair_action: Some(repair_plan.action()),
        repair_manual_reason: repair_plan
            .manual_reason()
            .map(RuntimeSnapshotCheckpointRepairManualReason::from),
    }
}

fn extract_turn_checkpoint_compaction_statuses(
    events: &[mvp::session::repository::SessionEventRecord],
) -> Vec<mvp::conversation::TurnCheckpointProgressStatus> {
    events
        .iter()
        .filter(|event| event.event_kind == "turn_checkpoint")
        .filter_map(|event| {
            event
                .payload_json
                .get("finalization_progress")
                .and_then(|progress| progress.get("compaction"))
                .and_then(serde_json::Value::as_str)
                .and_then(parse_compaction_status_label)
        })
        .collect()
}

fn parse_compaction_status_label(
    label: &str,
) -> Option<mvp::conversation::TurnCheckpointProgressStatus> {
    match label {
        "pending" => Some(mvp::conversation::TurnCheckpointProgressStatus::Pending),
        "skipped" => Some(mvp::conversation::TurnCheckpointProgressStatus::Skipped),
        "completed" => Some(mvp::conversation::TurnCheckpointProgressStatus::Completed),
        "failed" => Some(mvp::conversation::TurnCheckpointProgressStatus::Failed),
        "failed_open" => Some(mvp::conversation::TurnCheckpointProgressStatus::FailedOpen),
        _ => None,
    }
}

impl From<mvp::conversation::TurnCheckpointRepairManualReason>
    for RuntimeSnapshotCheckpointRepairManualReason
{
    fn from(value: mvp::conversation::TurnCheckpointRepairManualReason) -> Self {
        match value {
            mvp::conversation::TurnCheckpointRepairManualReason::CheckpointIdentityMissing => {
                Self::CheckpointIdentityMissing
            }
            mvp::conversation::TurnCheckpointRepairManualReason::SafeLaneBackpressureTerminalRequiresManualInspection => {
                Self::SafeLaneBackpressureTerminalRequiresManualInspection
            }
            mvp::conversation::TurnCheckpointRepairManualReason::SafeLaneSessionGovernorTerminalRequiresManualInspection => {
                Self::SafeLaneSessionGovernorTerminalRequiresManualInspection
            }
            mvp::conversation::TurnCheckpointRepairManualReason::CheckpointStateRequiresManualInspection => {
                Self::CheckpointStateRequiresManualInspection
            }
        }
    }
}

pub(crate) fn compaction_status_label(
    status: mvp::conversation::TurnCheckpointProgressStatus,
) -> &'static str {
    match status {
        mvp::conversation::TurnCheckpointProgressStatus::Pending => "pending",
        mvp::conversation::TurnCheckpointProgressStatus::Skipped => "skipped",
        mvp::conversation::TurnCheckpointProgressStatus::Completed => "completed",
        mvp::conversation::TurnCheckpointProgressStatus::Failed => "failed",
        mvp::conversation::TurnCheckpointProgressStatus::FailedOpen => "failed_open",
    }
}

pub(crate) fn is_failure_status(status: mvp::conversation::TurnCheckpointProgressStatus) -> bool {
    matches!(
        status,
        mvp::conversation::TurnCheckpointProgressStatus::Failed
            | mvp::conversation::TurnCheckpointProgressStatus::FailedOpen
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PrimaryLineageCheckpointProjection, RuntimeSnapshotCheckpointRepairManualReason,
        project_primary_lineage_checkpoint_events,
    };
    use crate::mvp;
    use serde_json::json;

    fn turn_checkpoint_event(
        id: i64,
        ts: i64,
        compaction: &str,
        stage: &str,
        identity_present: bool,
    ) -> mvp::session::repository::SessionEventRecord {
        let identity = if identity_present {
            json!({ "identity": { "session_id": "root-session" } })
        } else {
            json!({})
        };
        let checkpoint = json!({
            "lane": {
                "lane": "safe",
                "result_kind": "tool_call",
            },
            "finalization": {
                "persistence_mode": "success",
                "runs_after_turn": true,
                "attempts_context_compaction": true,
            }
        });
        let mut checkpoint_object = checkpoint.as_object().cloned().expect("checkpoint object");
        if identity_present {
            checkpoint_object.insert("identity".to_owned(), identity["identity"].clone());
        }

        mvp::session::repository::SessionEventRecord {
            id,
            session_id: "child-session".to_owned(),
            event_kind: "turn_checkpoint".to_owned(),
            actor_session_id: None,
            payload_json: json!({
                "schema_version": 1,
                "stage": stage,
                "checkpoint": checkpoint_object,
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": compaction,
                },
                "failure": null,
            }),
            ts,
        }
    }

    #[test]
    fn project_primary_lineage_checkpoint_events_uses_recent_sequence_for_retryable_repair() {
        let projection = project_primary_lineage_checkpoint_events(&[
            turn_checkpoint_event(1, 100, "failed", "finalization_failed", true),
            turn_checkpoint_event(2, 90, "completed", "finalized", true),
        ]);

        assert_eq!(
            projection.latest_compaction_status,
            Some(mvp::conversation::TurnCheckpointProgressStatus::Failed)
        );
        assert_eq!(projection.event_count, 2);
        assert_eq!(projection.failure_streak, 1);
        assert_eq!(
            projection.repair_action,
            Some(mvp::conversation::TurnCheckpointRecoveryAction::RunCompaction)
        );
        assert_eq!(projection.repair_manual_reason, None);
    }

    #[test]
    fn project_primary_lineage_checkpoint_events_marks_missing_identity_as_manual() {
        let projection = project_primary_lineage_checkpoint_events(&[turn_checkpoint_event(
            1,
            100,
            "pending",
            "post_persist",
            false,
        )]);

        assert_eq!(
            projection.latest_compaction_status,
            Some(mvp::conversation::TurnCheckpointProgressStatus::Pending)
        );
        assert_eq!(projection.event_count, 1);
        assert_eq!(projection.failure_streak, 0);
        assert_eq!(
            projection.repair_action,
            Some(mvp::conversation::TurnCheckpointRecoveryAction::InspectManually)
        );
        assert_eq!(
            projection.repair_manual_reason,
            Some(RuntimeSnapshotCheckpointRepairManualReason::CheckpointIdentityMissing)
        );
    }

    #[test]
    fn empty_primary_lineage_checkpoint_events_yield_default_projection() {
        let projection = project_primary_lineage_checkpoint_events(&[]);
        assert_eq!(projection.event_count, 0);
        assert_eq!(
            projection.latest_compaction_status,
            PrimaryLineageCheckpointProjection::default().latest_compaction_status
        );
        assert_eq!(projection.repair_action, None);
    }
}
