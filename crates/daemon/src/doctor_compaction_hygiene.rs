use std::path::Path;

use loong_app as mvp;
use serde_json::{Value, json};

use crate::doctor_cli::{DoctorCheck, DoctorCheckLevel};
use crate::runtime_snapshot_compaction_presentation::build_compaction_hygiene_json;

#[derive(Debug, Clone)]
pub(crate) struct DoctorCompactionHygieneSignal {
    pub(crate) enabled: bool,
    pub(crate) state: crate::RuntimeSnapshotCompactionHygieneState,
    pub(crate) check: DoctorCheck,
    pub(crate) next_steps: Vec<String>,
}

pub(crate) fn collect_doctor_compaction_hygiene_signal(
    config_path: &Path,
    config: &mvp::config::LoongConfig,
) -> DoctorCompactionHygieneSignal {
    let context_engine = match mvp::conversation::collect_context_engine_runtime_snapshot(config) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return DoctorCompactionHygieneSignal {
                enabled: false,
                state: crate::RuntimeSnapshotCompactionHygieneState::unavailable(
                    "unknown",
                    "turn_checkpoint",
                    Some(error.clone()),
                ),
                check: DoctorCheck {
                    name: "context compaction hygiene".to_owned(),
                    level: DoctorCheckLevel::Warn,
                    detail: format!("runtime snapshot unavailable: {error}"),
                },
                next_steps: vec![format!(
                    "Inspect runtime compaction hygiene evidence: {}",
                    runtime_snapshot_json_command(config_path)
                )],
            };
        }
    };

    let enabled = context_engine.compaction.enabled;
    let state = crate::runtime_snapshot_compaction_hygiene::collect_runtime_snapshot_compaction_hygiene_state(
        config,
        &context_engine,
    );
    compaction_hygiene_doctor_signal_from_state(&state, enabled, config_path)
}

fn compaction_hygiene_doctor_signal_from_state(
    state: &crate::RuntimeSnapshotCompactionHygieneState,
    enabled: bool,
    config_path: &Path,
) -> DoctorCompactionHygieneSignal {
    let assessment = state.assessment();
    let level = if !enabled {
        DoctorCheckLevel::Pass
    } else {
        match assessment.recovery_posture {
            crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Unavailable
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::ScopeLimited
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::InsufficientHistory
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::NoCompactionEvidence
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Watch
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::AutoRepairing
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Fragile => DoctorCheckLevel::Warn,
            crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::RetryExhausted
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::ManualLane
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Broken => DoctorCheckLevel::Fail,
            crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Idle
            | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Healthy => DoctorCheckLevel::Pass,
            crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Unknown => DoctorCheckLevel::Warn,
        }
    };

    let detail = format!(
        "enabled={} posture={} continuity={} repairability={} recovery_posture={} scope={} root={} checkpoint_events={} failed_open_rate={} diagnostics_coverage={}",
        enabled,
        assessment.posture.as_str(),
        assessment.continuity_health.as_str(),
        assessment.continuity_repairability.as_str(),
        assessment.recovery_posture.as_str(),
        assessment.trend_scope.as_str(),
        state
            .primary_lineage
            .root_session_id
            .as_deref()
            .unwrap_or("-"),
        state.primary_lineage.checkpoint_event_count,
        state.failed_open_rate_summary(),
        state.diagnostics_coverage_summary(),
    );

    let mut next_steps = Vec::new();
    let runtime_snapshot_command = runtime_snapshot_json_command(config_path);
    let sessions_command = crate::cli_handoff::format_subcommand_with_config(
        "sessions",
        &config_path.display().to_string(),
    );

    match assessment.recovery_posture {
        crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::RetryExhausted
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::ManualLane
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Broken => {
            next_steps.push(format!(
                "Inspect runtime compaction hygiene evidence: {runtime_snapshot_command}"
            ));
            next_steps.push(format!(
                "Review recent session checkpoint summaries and continuity signals: {sessions_command}"
            ));
        }
        crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Watch
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::AutoRepairing
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Fragile
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::ScopeLimited
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::InsufficientHistory
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::NoCompactionEvidence
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Unknown => {
            next_steps.push(format!(
                "Inspect runtime compaction hygiene evidence: {runtime_snapshot_command}"
            ));
        }
        crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Unavailable
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Idle
        | crate::runtime_snapshot_compaction_assessment::CompactionRecoveryPosture::Healthy => {}
    }

    DoctorCompactionHygieneSignal {
        enabled,
        state: state.clone(),
        check: DoctorCheck {
            name: "context compaction hygiene".to_owned(),
            level,
            detail,
        },
        next_steps,
    }
}

pub(crate) fn doctor_compaction_hygiene_json_payload(
    signal: &DoctorCompactionHygieneSignal,
) -> Value {
    json!({
        "enabled": signal.enabled,
        "level": match signal.check.level {
            DoctorCheckLevel::Pass => "ok",
            DoctorCheckLevel::Warn => "warn",
            DoctorCheckLevel::Fail => "fail",
        },
        "detail": signal.check.detail,
        "next_steps": signal.next_steps,
        "signal": build_compaction_hygiene_json(&signal.state),
    })
}

fn runtime_snapshot_json_command(config_path: &Path) -> String {
    format!(
        "{} runtime snapshot --json --config {}",
        mvp::config::CLI_COMMAND_NAME,
        crate::cli_handoff::shell_quote_argument(&config_path.display().to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        compaction_hygiene_doctor_signal_from_state, doctor_compaction_hygiene_json_payload,
    };
    use crate::RuntimeSnapshotCompactionHygieneState;
    use crate::doctor_cli::DoctorCheckLevel;
    use crate::mvp::conversation::{TurnCheckpointProgressStatus, TurnCheckpointRecoveryAction};
    use crate::runtime_snapshot_compaction_hygiene::{
        RuntimeSnapshotCompactionHygieneWindow, RuntimeSnapshotCompactionLineageState,
    };

    fn sample_retry_exhausted_compaction_hygiene_state() -> RuntimeSnapshotCompactionHygieneState {
        RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "ok".to_owned(),
            trend_scope: "primary_lineage".to_owned(),
            primary_lineage: RuntimeSnapshotCompactionLineageState {
                root_session_id: Some("root-session".to_owned()),
                sampled_session_count: 2,
                compaction_sample_count: 2,
                latest_compaction_status: Some(TurnCheckpointProgressStatus::FailedOpen),
                compaction_failure_streak: 1,
                checkpoint_event_count: 3,
                checkpoint_failure_streak: 2,
                checkpoint_repair_action: Some(TurnCheckpointRecoveryAction::RunCompaction),
                checkpoint_repair_manual_reason: None,
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
    fn compaction_hygiene_doctor_signal_fails_manual_lane_and_guides_runtime_snapshot_and_sessions()
    {
        let signal = compaction_hygiene_doctor_signal_from_state(
            &sample_retry_exhausted_compaction_hygiene_state(),
            true,
            std::path::Path::new("/tmp/loong.toml"),
        );

        assert_eq!(signal.check.level, DoctorCheckLevel::Fail);
        assert!(signal.check.detail.contains("continuity=broken"));
        assert!(
            signal
                .check
                .detail
                .contains("recovery_posture=retry_exhausted")
        );
        assert!(signal.next_steps.iter().any(|step| {
            step == "Inspect runtime compaction hygiene evidence: loong runtime snapshot --json --config '/tmp/loong.toml'"
        }));
        assert!(signal.next_steps.iter().any(|step| {
            step == "Review recent session checkpoint summaries and continuity signals: loong sessions --config '/tmp/loong.toml'"
        }));
    }

    #[test]
    fn compaction_hygiene_doctor_signal_passes_when_compaction_is_disabled() {
        let signal = compaction_hygiene_doctor_signal_from_state(
            &crate::RuntimeSnapshotCompactionHygieneState::unknown_unavailable(),
            false,
            std::path::Path::new("/tmp/loong.toml"),
        );

        assert_eq!(signal.check.level, DoctorCheckLevel::Pass);
        assert!(signal.check.detail.contains("enabled=false"));
        assert!(signal.next_steps.is_empty());
    }

    #[test]
    fn doctor_compaction_hygiene_json_payload_surfaces_structured_signal() {
        let signal = compaction_hygiene_doctor_signal_from_state(
            &sample_retry_exhausted_compaction_hygiene_state(),
            true,
            std::path::Path::new("/tmp/loong.toml"),
        );

        let payload = doctor_compaction_hygiene_json_payload(&signal);

        assert_eq!(payload["enabled"], serde_json::json!(true));
        assert_eq!(payload["level"], serde_json::json!("fail"));
        assert_eq!(
            payload["signal"]["metrics"]["continuity_health"],
            serde_json::json!("broken")
        );
        assert_eq!(
            payload["signal"]["metrics"]["continuity_repairability"],
            serde_json::json!("retryable")
        );
        assert_eq!(
            payload["signal"]["metrics"]["recovery_posture"],
            serde_json::json!("retry_exhausted")
        );
        assert!(
            payload["next_steps"]
                .as_array()
                .expect("next_steps array")
                .iter()
                .any(|step| step.as_str() == Some(
                    "Inspect runtime compaction hygiene evidence: loong runtime snapshot --json --config '/tmp/loong.toml'"
                ))
        );
    }
}
