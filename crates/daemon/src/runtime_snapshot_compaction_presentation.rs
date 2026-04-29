use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::RuntimeSnapshotCompactionHygieneState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeSnapshotCompactionHygieneStatusValues {
    pub(crate) hygiene: String,
    pub(crate) samples: String,
    pub(crate) prunes: String,
    pub(crate) pressure: String,
    pub(crate) trend: String,
    pub(crate) repairability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuntimeSnapshotCompactionHygieneMetricsDocument {
    pub posture: String,
    pub sample_order: String,
    pub trend_scope: String,
    pub continuity_source: String,
    pub continuity_health: String,
    pub continuity_repairability: String,
    pub recovery_posture: String,
    pub reliability_trend: String,
    pub coverage_trend: String,
    pub pressure_trend: String,
    pub diagnostics_coverage_milli: Option<u32>,
    pub failed_open_rate_milli: Option<u32>,
    pub demoted_recent_turns_per_diagnostic_session_milli: Option<u32>,
    pub low_signal_turns_per_diagnostic_session_milli: Option<u32>,
    pub tool_result_prunes_per_diagnostic_session_milli: Option<u32>,
    pub tool_outcome_prunes_per_diagnostic_session_milli: Option<u32>,
}

pub(crate) fn render_runtime_snapshot_compaction_lines(
    state: &RuntimeSnapshotCompactionHygieneState,
) -> Vec<String> {
    let assessment = state.assessment();
    vec![
        format!(
            "context_engine compaction_hygiene evidence_status={} posture={} sampled_sessions={} sessions_with_diagnostics={} missing_evidence={} read_errors={} diagnostics_coverage={} failed_open_sessions={} failed_open_rate={} demoted_recent_turns={} low_signal_turns={} tool_result_prunes={} tool_outcome_prunes={}",
            state.evidence_status,
            assessment.posture.as_str(),
            state.sampled_session_count(),
            state.sessions_with_diagnostics(),
            state.sampled_sessions_without_diagnostics(),
            state.sampled_session_read_errors(),
            state.diagnostics_coverage_summary(),
            state.failed_open_session_count(),
            state.failed_open_rate_summary(),
            state.total_demoted_recent_turns(),
            state.total_low_signal_turns(),
            state.total_tool_result_prunes(),
            state.total_tool_outcome_prunes(),
        ),
        format!(
            "context_engine compaction_pressure demoted_recent_avg={} low_signal_avg={} tool_result_prunes_avg={} tool_outcome_prunes_avg={}",
            state.demoted_recent_turns_pressure_summary(),
            state.low_signal_turns_pressure_summary(),
            state.tool_result_prunes_pressure_summary(),
            state.tool_outcome_prunes_pressure_summary(),
        ),
        format!(
            "context_engine compaction_trend scope={} lineage_root={} lineage_samples={} latest_compaction={} session_failure_streak={} checkpoint_events={} checkpoint_failure_streak={} continuity_source={} continuity={} sample_order={} recent_window={} baseline_window={} reliability={} coverage={} pressure={}",
            assessment.trend_scope.as_str(),
            state
                .primary_lineage
                .root_session_id
                .as_deref()
                .unwrap_or("-"),
            state.primary_lineage.sampled_session_count,
            state.primary_lineage_latest_compaction_status_label(),
            state.primary_lineage.compaction_failure_streak,
            state.primary_lineage.checkpoint_event_count,
            state.primary_lineage.checkpoint_failure_streak,
            assessment.continuity_source.as_str(),
            assessment.continuity_health.as_str(),
            assessment.sample_order.as_str(),
            state.recent_window.sampled_session_count,
            state.baseline_window.sampled_session_count,
            state.reliability_trend(),
            state.coverage_trend(),
            state.pressure_trend(),
        ),
        format!(
            "context_engine compaction_repairability repairability={} recovery_posture={} action={} manual_reason={}",
            assessment.continuity_repairability.as_str(),
            assessment.recovery_posture.as_str(),
            state.continuity_repair_action(),
            state.continuity_repair_manual_reason(),
        ),
    ]
}

pub(crate) fn build_compaction_hygiene_status_values(
    state: &RuntimeSnapshotCompactionHygieneState,
) -> RuntimeSnapshotCompactionHygieneStatusValues {
    let assessment = state.assessment();
    RuntimeSnapshotCompactionHygieneStatusValues {
        hygiene: format!(
            "{} · posture={} · surface={} · evidence={} · coverage={}",
            state.strategy,
            assessment.posture.as_str(),
            state.diagnostics_surface,
            state.evidence_status,
            state.diagnostics_coverage_summary(),
        ),
        samples: format!(
            "sampled={} with_diagnostics={} missing_evidence={} read_errors={} failed_open={} rate={}",
            state.sampled_session_count(),
            state.sessions_with_diagnostics(),
            state.sampled_sessions_without_diagnostics(),
            state.sampled_session_read_errors(),
            state.failed_open_session_count(),
            state.failed_open_rate_summary(),
        ),
        prunes: format!(
            "demoted_recent={} low_signal={} tool_results={} tool_outcomes={}",
            state.total_demoted_recent_turns(),
            state.total_low_signal_turns(),
            state.total_tool_result_prunes(),
            state.total_tool_outcome_prunes(),
        ),
        pressure: format!(
            "demoted_recent={} low_signal={} tool_results={} tool_outcomes={}",
            state.demoted_recent_turns_pressure_summary(),
            state.low_signal_turns_pressure_summary(),
            state.tool_result_prunes_pressure_summary(),
            state.tool_outcome_prunes_pressure_summary(),
        ),
        trend: format!(
            "{} · scope={} root={} lineage_samples={} latest={} session_failure_streak={} checkpoint_events={} checkpoint_failure_streak={} continuity_source={} continuity={} · recent_window={} baseline_window={} · reliability={} coverage={} pressure={}",
            assessment.sample_order.as_str(),
            assessment.trend_scope.as_str(),
            state
                .primary_lineage
                .root_session_id
                .as_deref()
                .unwrap_or("-"),
            state.primary_lineage.sampled_session_count,
            state.primary_lineage_latest_compaction_status_label(),
            state.primary_lineage.compaction_failure_streak,
            state.primary_lineage.checkpoint_event_count,
            state.primary_lineage.checkpoint_failure_streak,
            assessment.continuity_source.as_str(),
            assessment.continuity_health.as_str(),
            state.recent_window.sampled_session_count,
            state.baseline_window.sampled_session_count,
            state.reliability_trend(),
            state.coverage_trend(),
            state.pressure_trend(),
        ),
        repairability: format!(
            "{} · recovery_posture={} · action={} · manual_reason={}",
            assessment.continuity_repairability.as_str(),
            assessment.recovery_posture.as_str(),
            state.continuity_repair_action(),
            state.continuity_repair_manual_reason(),
        ),
    }
}

pub(crate) fn build_compaction_hygiene_metrics_document(
    state: &RuntimeSnapshotCompactionHygieneState,
) -> RuntimeSnapshotCompactionHygieneMetricsDocument {
    let assessment = state.assessment();
    RuntimeSnapshotCompactionHygieneMetricsDocument {
        posture: assessment.posture.as_str().to_owned(),
        sample_order: assessment.sample_order.as_str().to_owned(),
        trend_scope: assessment.trend_scope.as_str().to_owned(),
        continuity_source: assessment.continuity_source.as_str().to_owned(),
        continuity_health: assessment.continuity_health.as_str().to_owned(),
        continuity_repairability: assessment.continuity_repairability.as_str().to_owned(),
        recovery_posture: assessment.recovery_posture.as_str().to_owned(),
        reliability_trend: assessment.reliability_trend.as_str().to_owned(),
        coverage_trend: assessment.coverage_trend.as_str().to_owned(),
        pressure_trend: assessment.pressure_trend.as_str().to_owned(),
        diagnostics_coverage_milli: state.diagnostics_coverage_milli(),
        failed_open_rate_milli: state.failed_open_rate_milli(),
        demoted_recent_turns_per_diagnostic_session_milli: state
            .demoted_recent_turns_per_diagnostic_session_milli(),
        low_signal_turns_per_diagnostic_session_milli: state
            .low_signal_turns_per_diagnostic_session_milli(),
        tool_result_prunes_per_diagnostic_session_milli: state
            .tool_result_prunes_per_diagnostic_session_milli(),
        tool_outcome_prunes_per_diagnostic_session_milli: state
            .tool_outcome_prunes_per_diagnostic_session_milli(),
    }
}

pub(crate) fn build_compaction_hygiene_json(
    state: &RuntimeSnapshotCompactionHygieneState,
) -> Value {
    json!({
        "strategy": state.strategy.clone(),
        "diagnostics_surface": state.diagnostics_surface.clone(),
        "evidence_status": state.evidence_status.clone(),
        "trend_scope": state.trend_scope(),
        "primary_lineage_root_session_id": state.primary_lineage.root_session_id.clone(),
        "primary_lineage_sampled_session_count": state.primary_lineage.sampled_session_count,
        "primary_lineage_compaction_sample_count": state.primary_lineage.compaction_sample_count,
        "primary_lineage_latest_compaction_status": state.primary_lineage.latest_compaction_status,
        "primary_lineage_compaction_failure_streak": state.primary_lineage.compaction_failure_streak,
        "primary_lineage_checkpoint_event_count": state.primary_lineage.checkpoint_event_count,
        "primary_lineage_checkpoint_failure_streak": state.primary_lineage.checkpoint_failure_streak,
        "primary_lineage_checkpoint_repair_action": state.primary_lineage.checkpoint_repair_action,
        "primary_lineage_checkpoint_repair_manual_reason": state.primary_lineage.checkpoint_repair_manual_reason,
        "sampled_session_count": state.sampled_session_count(),
        "sessions_with_diagnostics": state.sessions_with_diagnostics(),
        "sampled_sessions_without_diagnostics": state.sampled_sessions_without_diagnostics(),
        "sampled_session_read_errors": state.sampled_session_read_errors(),
        "failed_open_session_count": state.failed_open_session_count(),
        "total_demoted_recent_turns": state.total_demoted_recent_turns(),
        "total_low_signal_turns": state.total_low_signal_turns(),
        "total_tool_result_prunes": state.total_tool_result_prunes(),
        "total_tool_outcome_prunes": state.total_tool_outcome_prunes(),
        "recent_window": state.recent_window,
        "baseline_window": state.baseline_window,
        "error": state.error,
        "metrics": build_compaction_hygiene_metrics_document(state),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_compaction_hygiene_json, build_compaction_hygiene_status_values,
        render_runtime_snapshot_compaction_lines,
    };
    use crate::RuntimeSnapshotCompactionHygieneState;
    use crate::mvp::conversation::{TurnCheckpointProgressStatus, TurnCheckpointRecoveryAction};
    use crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionHygieneWindow;
    use crate::runtime_snapshot_compaction_sequence::RuntimeSnapshotCheckpointRepairManualReason;

    fn fixture() -> RuntimeSnapshotCompactionHygieneState {
        RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "ok".to_owned(),
            trend_scope: "primary_lineage".to_owned(),
            primary_lineage:
                crate::runtime_snapshot_compaction_hygiene::RuntimeSnapshotCompactionLineageState {
                    root_session_id: Some("root-session".to_owned()),
                    sampled_session_count: 2,
                    compaction_sample_count: 2,
                    latest_compaction_status: Some(TurnCheckpointProgressStatus::FailedOpen),
                    compaction_failure_streak: 1,
                    checkpoint_event_count: 3,
                    checkpoint_failure_streak: 2,
                    checkpoint_repair_action: Some(TurnCheckpointRecoveryAction::RunCompaction),
                    checkpoint_repair_manual_reason: Some(
                        RuntimeSnapshotCheckpointRepairManualReason::CheckpointStateRequiresManualInspection,
                    ),
                },
            overall_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 4,
                sessions_with_diagnostics: 2,
                sampled_session_read_errors: 0,
                failed_open_session_count: 1,
                total_demoted_recent_turns: 3,
                total_low_signal_turns: 4,
                total_tool_result_prunes: 2,
                total_tool_outcome_prunes: 1,
            },
            recent_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 2,
                sessions_with_diagnostics: 1,
                sampled_session_read_errors: 0,
                failed_open_session_count: 1,
                total_demoted_recent_turns: 2,
                total_low_signal_turns: 2,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 0,
            },
            baseline_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 2,
                sessions_with_diagnostics: 1,
                sampled_session_read_errors: 0,
                failed_open_session_count: 0,
                total_demoted_recent_turns: 1,
                total_low_signal_turns: 2,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 1,
            },
            error: None,
        }
    }

    #[test]
    fn compaction_hygiene_presentation_surfaces_shared_status_trend_and_repairability_strings() {
        let state = fixture();
        let values = build_compaction_hygiene_status_values(&state);
        let lines = render_runtime_snapshot_compaction_lines(&state);
        let json = build_compaction_hygiene_json(&state);

        assert!(values.hygiene.contains("posture=degraded"));
        assert!(values.samples.contains("failed_open=1"));
        assert!(values.trend.contains("scope=primary_lineage"));
        assert!(values.trend.contains("continuity=broken"));
        assert!(
            values
                .repairability
                .contains("recovery_posture=retry_exhausted")
        );
        assert!(values.repairability.contains("action=run_compaction"));
        assert_eq!(lines.len(), 4);
        assert!(lines[2].contains("checkpoint_events=3"));
        assert!(lines[3].contains("manual_reason=checkpoint_state_requires_manual_inspection"));
        assert_eq!(
            json["metrics"]["continuity_repairability"],
            serde_json::json!("retryable")
        );
        assert_eq!(
            json["primary_lineage_checkpoint_repair_action"],
            serde_json::json!("run_compaction")
        );
        assert_eq!(
            json["primary_lineage_root_session_id"],
            serde_json::json!("root-session")
        );
    }
}
