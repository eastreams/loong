use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mvp;
use crate::runtime_snapshot_compaction_assessment::{
    RuntimeSnapshotCompactionAssessment, assess_compaction_hygiene,
};
use crate::runtime_snapshot_compaction_sequence::{
    RuntimeSnapshotCheckpointRepairManualReason, collect_primary_lineage_checkpoint_projection,
    compaction_status_label, is_failure_status,
};

const RUNTIME_SNAPSHOT_COMPACTION_HYGIENE_SESSION_LIMIT: usize = 8;
const COMPACTION_HYGIENE_TREND_TOLERANCE_MILLI: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeSnapshotCompactionHygieneWindow {
    pub sampled_session_count: usize,
    pub sessions_with_diagnostics: usize,
    #[serde(default)]
    pub sampled_session_read_errors: usize,
    pub failed_open_session_count: usize,
    pub total_demoted_recent_turns: u64,
    pub total_low_signal_turns: u64,
    pub total_tool_result_prunes: u64,
    pub total_tool_outcome_prunes: u64,
}

impl RuntimeSnapshotCompactionHygieneWindow {
    pub fn sampled_sessions_without_diagnostics(&self) -> usize {
        self.sampled_session_count
            .saturating_sub(self.sessions_with_diagnostics)
            .saturating_sub(self.sampled_session_read_errors)
    }

    pub fn diagnostics_coverage_milli(&self) -> Option<u32> {
        milli_ratio(
            self.sessions_with_diagnostics as u64,
            self.sampled_session_count as u64,
        )
    }

    pub fn failed_open_rate_milli(&self) -> Option<u32> {
        milli_ratio(
            self.failed_open_session_count as u64,
            self.sessions_with_diagnostics as u64,
        )
    }

    pub fn pressure_total_events(&self) -> u64 {
        self.total_demoted_recent_turns
            .saturating_add(self.total_low_signal_turns)
            .saturating_add(self.total_tool_result_prunes)
            .saturating_add(self.total_tool_outcome_prunes)
    }

    pub fn pressure_per_diagnostic_session_milli(&self) -> Option<u32> {
        milli_ratio(
            self.pressure_total_events(),
            self.sessions_with_diagnostics as u64,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeSnapshotCompactionLineageState {
    #[serde(default)]
    pub root_session_id: Option<String>,
    #[serde(default)]
    pub sampled_session_count: usize,
    #[serde(default)]
    pub compaction_sample_count: usize,
    #[serde(default)]
    pub latest_compaction_status: Option<mvp::conversation::TurnCheckpointProgressStatus>,
    #[serde(default)]
    pub compaction_failure_streak: usize,
    #[serde(default)]
    pub checkpoint_event_count: usize,
    #[serde(default)]
    pub checkpoint_failure_streak: usize,
    #[serde(default)]
    pub checkpoint_repair_action: Option<mvp::conversation::TurnCheckpointRecoveryAction>,
    #[serde(default)]
    pub checkpoint_repair_manual_reason: Option<RuntimeSnapshotCheckpointRepairManualReason>,
}

impl RuntimeSnapshotCompactionLineageState {
    pub fn latest_compaction_status_label(&self) -> &str {
        self.latest_compaction_status
            .map(compaction_status_label)
            .unwrap_or("-")
    }

    pub fn repair_action_label(&self) -> &str {
        self.checkpoint_repair_action
            .map(mvp::conversation::TurnCheckpointRecoveryAction::as_str)
            .unwrap_or("-")
    }

    pub fn repair_manual_reason_label(&self) -> &str {
        self.checkpoint_repair_manual_reason
            .map(RuntimeSnapshotCheckpointRepairManualReason::as_str)
            .unwrap_or("-")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSnapshotCompactionHygieneState {
    pub strategy: String,
    pub diagnostics_surface: String,
    pub evidence_status: String,
    #[serde(default)]
    pub trend_scope: String,
    #[serde(default)]
    pub primary_lineage: RuntimeSnapshotCompactionLineageState,
    #[serde(flatten)]
    pub overall_window: RuntimeSnapshotCompactionHygieneWindow,
    #[serde(default)]
    pub recent_window: RuntimeSnapshotCompactionHygieneWindow,
    #[serde(default)]
    pub baseline_window: RuntimeSnapshotCompactionHygieneWindow,
    pub error: Option<String>,
}

impl RuntimeSnapshotCompactionHygieneState {
    pub(crate) fn unavailable(
        strategy: impl Into<String>,
        diagnostics_surface: impl Into<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            strategy: strategy.into(),
            diagnostics_surface: diagnostics_surface.into(),
            evidence_status: "unavailable".to_owned(),
            trend_scope: "unavailable".to_owned(),
            primary_lineage: RuntimeSnapshotCompactionLineageState::default(),
            overall_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            recent_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            baseline_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            error,
        }
    }

    pub(crate) fn unknown_unavailable() -> Self {
        Self::unavailable("unknown", "turn_checkpoint", None)
    }

    pub(crate) fn decode_or_unknown(value: Option<&Value>) -> Self {
        value
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_else(Self::unknown_unavailable)
    }

    pub fn assessment(&self) -> RuntimeSnapshotCompactionAssessment {
        assess_compaction_hygiene(self)
    }

    pub const fn sample_order(&self) -> &'static str {
        "updated_at_desc"
    }

    pub const fn continuity_source(&self) -> &'static str {
        "session_events_recent"
    }

    pub fn continuity_repairability(&self) -> &'static str {
        self.assessment().continuity_repairability.as_str()
    }

    pub fn recovery_posture(&self) -> &'static str {
        self.assessment().recovery_posture.as_str()
    }

    pub fn continuity_repair_action(&self) -> &str {
        self.primary_lineage.repair_action_label()
    }

    pub fn continuity_repair_manual_reason(&self) -> &str {
        self.primary_lineage.repair_manual_reason_label()
    }

    pub fn primary_lineage_latest_compaction_status_label(&self) -> &str {
        self.primary_lineage.latest_compaction_status_label()
    }

    pub fn sampled_session_count(&self) -> usize {
        self.overall_window.sampled_session_count
    }

    pub fn sessions_with_diagnostics(&self) -> usize {
        self.overall_window.sessions_with_diagnostics
    }

    pub fn sampled_session_read_errors(&self) -> usize {
        self.overall_window.sampled_session_read_errors
    }

    pub fn failed_open_session_count(&self) -> usize {
        self.overall_window.failed_open_session_count
    }

    pub fn total_demoted_recent_turns(&self) -> u64 {
        self.overall_window.total_demoted_recent_turns
    }

    pub fn total_low_signal_turns(&self) -> u64 {
        self.overall_window.total_low_signal_turns
    }

    pub fn total_tool_result_prunes(&self) -> u64 {
        self.overall_window.total_tool_result_prunes
    }

    pub fn total_tool_outcome_prunes(&self) -> u64 {
        self.overall_window.total_tool_outcome_prunes
    }

    pub fn trend_scope(&self) -> &str {
        if self.trend_scope.is_empty() {
            if self.overall_window.sampled_session_count == 0 {
                "idle"
            } else {
                "recent_sessions"
            }
        } else {
            self.trend_scope.as_str()
        }
    }

    pub fn continuity_health(&self) -> &'static str {
        self.assessment().continuity_health.as_str()
    }

    pub fn posture(&self) -> &'static str {
        self.assessment().posture.as_str()
    }

    pub fn sampled_sessions_without_diagnostics(&self) -> usize {
        self.overall_window.sampled_sessions_without_diagnostics()
    }

    pub fn reliability_trend(&self) -> &'static str {
        compare_trend_milli(
            self.recent_window.failed_open_rate_milli(),
            self.baseline_window.failed_open_rate_milli(),
            TrendDirectionMode::HigherIsWorse,
        )
    }

    pub fn coverage_trend(&self) -> &'static str {
        compare_trend_milli(
            self.recent_window.diagnostics_coverage_milli(),
            self.baseline_window.diagnostics_coverage_milli(),
            TrendDirectionMode::HigherIsBetter,
        )
    }

    pub fn pressure_trend(&self) -> &'static str {
        compare_trend_milli(
            self.recent_window.pressure_per_diagnostic_session_milli(),
            self.baseline_window.pressure_per_diagnostic_session_milli(),
            TrendDirectionMode::HigherIsWorse,
        )
    }

    pub fn trend_summary(&self) -> String {
        format!(
            "scope={} reliability={} coverage={} pressure={}",
            self.trend_scope(),
            self.reliability_trend(),
            self.coverage_trend(),
            self.pressure_trend()
        )
    }

    pub fn diagnostics_coverage_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.sessions_with_diagnostics as u64,
            self.overall_window.sampled_session_count as u64,
        )
    }

    pub fn failed_open_rate_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.failed_open_session_count as u64,
            self.overall_window.sessions_with_diagnostics as u64,
        )
    }

    pub fn demoted_recent_turns_per_diagnostic_session_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.total_demoted_recent_turns,
            self.overall_window.sessions_with_diagnostics as u64,
        )
    }

    pub fn low_signal_turns_per_diagnostic_session_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.total_low_signal_turns,
            self.overall_window.sessions_with_diagnostics as u64,
        )
    }

    pub fn tool_result_prunes_per_diagnostic_session_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.total_tool_result_prunes,
            self.overall_window.sessions_with_diagnostics as u64,
        )
    }

    pub fn tool_outcome_prunes_per_diagnostic_session_milli(&self) -> Option<u32> {
        milli_ratio(
            self.overall_window.total_tool_outcome_prunes,
            self.overall_window.sessions_with_diagnostics as u64,
        )
    }

    pub fn diagnostics_coverage_summary(&self) -> String {
        format_ratio_rollup(
            self.overall_window.sessions_with_diagnostics as u64,
            self.overall_window.sampled_session_count as u64,
            self.diagnostics_coverage_milli(),
        )
    }

    pub fn failed_open_rate_summary(&self) -> String {
        format_ratio_rollup(
            self.overall_window.failed_open_session_count as u64,
            self.overall_window.sessions_with_diagnostics as u64,
            self.failed_open_rate_milli(),
        )
    }

    pub fn demoted_recent_turns_pressure_summary(&self) -> String {
        format_pressure_rollup(self.demoted_recent_turns_per_diagnostic_session_milli())
    }

    pub fn low_signal_turns_pressure_summary(&self) -> String {
        format_pressure_rollup(self.low_signal_turns_per_diagnostic_session_milli())
    }

    pub fn tool_result_prunes_pressure_summary(&self) -> String {
        format_pressure_rollup(self.tool_result_prunes_per_diagnostic_session_milli())
    }

    pub fn tool_outcome_prunes_pressure_summary(&self) -> String {
        format_pressure_rollup(self.tool_outcome_prunes_per_diagnostic_session_milli())
    }
}

pub(crate) fn collect_runtime_snapshot_compaction_hygiene_state(
    config: &mvp::config::LoongConfig,
    context_engine: &mvp::conversation::ContextEngineRuntimeSnapshot,
) -> RuntimeSnapshotCompactionHygieneState {
    let strategy = context_engine.compaction.hygiene_strategy().to_owned();
    let diagnostics_surface = context_engine.compaction.diagnostics_surface().to_owned();
    let memory_config = mvp::session::store::SessionStoreConfig::from_memory_config(&config.memory);
    let repository = match mvp::session::repository::SessionRepository::new(&memory_config) {
        Ok(repository) => repository,
        Err(error) => {
            return unavailable_state(strategy, diagnostics_surface, error);
        }
    };
    let sessions = match repository.list_sessions() {
        Ok(sessions) => sessions,
        Err(error) => {
            return unavailable_state(strategy, diagnostics_surface, error);
        }
    };

    let sampled_sessions = sessions
        .into_iter()
        .take(RUNTIME_SNAPSHOT_COMPACTION_HYGIENE_SESSION_LIMIT)
        .collect::<Vec<_>>();
    let transcript_page_size = config
        .memory
        .sliding_window
        .saturating_mul(4)
        .clamp(16, 128);
    let mut samples = Vec::with_capacity(sampled_sessions.len());

    for session in &sampled_sessions {
        let mut sample = CompactionHygieneSample {
            session_id: session.session_id.clone(),
            lineage_root_session_id: repository
                .lineage_root_session_id(session.session_id.as_str())
                .ok()
                .flatten(),
            ..CompactionHygieneSample::default()
        };
        let turns = match mvp::session::store::transcript_session_turns_paged(
            session.session_id.as_str(),
            transcript_page_size,
            &memory_config,
        ) {
            Ok(turns) => turns,
            Err(_error) => {
                sample.read_error = true;
                samples.push(sample);
                continue;
            }
        };
        let assistant_contents = turns
            .iter()
            .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str()));
        let summary = mvp::conversation::summarize_turn_checkpoint_events(assistant_contents);
        sample.latest_compaction_status = summary.latest_compaction;
        let Some(diagnostics) = summary.latest_compaction_diagnostics.as_ref() else {
            samples.push(sample);
            continue;
        };

        sample.has_diagnostics = true;
        sample.failed_open = summary.latest_compaction
            == Some(mvp::conversation::TurnCheckpointProgressStatus::FailedOpen);
        sample.demoted_recent_turns = diagnostics.demoted_recent_turn_count as u64;
        sample.low_signal_turns = diagnostics.low_signal_turns as u64;
        sample.tool_result_prunes = diagnostics.tool_result_line_prunes as u64;
        sample.tool_outcome_prunes = diagnostics.tool_outcome_record_prunes as u64;
        samples.push(sample);
    }

    let total_window = aggregate_window(samples.as_slice());
    let primary_lineage_root_session_id = samples
        .first()
        .and_then(|sample| sample.lineage_root_session_id.clone());
    let primary_lineage_samples = primary_lineage_root_session_id
        .as_ref()
        .map(|root| {
            samples
                .iter()
                .filter(|sample| sample.lineage_root_session_id.as_deref() == Some(root.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let primary_lineage_compaction_sample_count = primary_lineage_samples
        .iter()
        .filter(|sample| sample.latest_compaction_status.is_some())
        .count();
    let primary_lineage_compaction_failure_streak = primary_lineage_samples
        .iter()
        .take_while(|sample| {
            sample
                .latest_compaction_status
                .is_some_and(is_failure_status)
        })
        .count();
    let primary_lineage_session_ids = primary_lineage_samples
        .iter()
        .map(|sample| sample.session_id.clone())
        .collect::<Vec<_>>();
    let primary_lineage_checkpoint_projection = collect_primary_lineage_checkpoint_projection(
        &repository,
        primary_lineage_session_ids.as_slice(),
    );
    let primary_lineage = RuntimeSnapshotCompactionLineageState {
        root_session_id: primary_lineage_root_session_id.clone(),
        sampled_session_count: primary_lineage_samples.len(),
        compaction_sample_count: primary_lineage_compaction_sample_count,
        latest_compaction_status: primary_lineage_checkpoint_projection.latest_compaction_status,
        compaction_failure_streak: primary_lineage_compaction_failure_streak,
        checkpoint_event_count: primary_lineage_checkpoint_projection.event_count,
        checkpoint_failure_streak: primary_lineage_checkpoint_projection.failure_streak,
        checkpoint_repair_action: primary_lineage_checkpoint_projection.repair_action,
        checkpoint_repair_manual_reason: primary_lineage_checkpoint_projection.repair_manual_reason,
    };
    let use_primary_lineage_for_trends = primary_lineage_samples.len() >= 2;
    let trend_samples = if use_primary_lineage_for_trends {
        primary_lineage_samples.as_slice()
    } else {
        samples.as_slice()
    };
    let split_index = trend_samples.len().div_ceil(2);
    let (recent_samples, baseline_samples) = trend_samples.split_at(split_index);
    let recent_window = aggregate_window(recent_samples);
    let baseline_window = aggregate_window(baseline_samples);
    let trend_scope = if sampled_sessions.is_empty() {
        "idle"
    } else if use_primary_lineage_for_trends {
        "primary_lineage"
    } else if primary_lineage_root_session_id.is_some() {
        "recent_sessions_fallback"
    } else {
        "recent_sessions"
    };
    let evidence_status = resolve_compaction_hygiene_evidence_status(
        sampled_sessions.len(),
        total_window.sessions_with_diagnostics,
        total_window.sampled_session_read_errors,
    );

    RuntimeSnapshotCompactionHygieneState {
        strategy,
        diagnostics_surface,
        evidence_status: evidence_status.to_owned(),
        trend_scope: trend_scope.to_owned(),
        primary_lineage,
        overall_window: total_window,
        recent_window,
        baseline_window,
        error: None,
    }
}

fn unavailable_state(
    strategy: String,
    diagnostics_surface: String,
    error: String,
) -> RuntimeSnapshotCompactionHygieneState {
    RuntimeSnapshotCompactionHygieneState::unavailable(strategy, diagnostics_surface, Some(error))
}

#[derive(Debug, Clone, Default)]
struct CompactionHygieneSample {
    session_id: String,
    lineage_root_session_id: Option<String>,
    latest_compaction_status: Option<mvp::conversation::TurnCheckpointProgressStatus>,
    has_diagnostics: bool,
    read_error: bool,
    failed_open: bool,
    demoted_recent_turns: u64,
    low_signal_turns: u64,
    tool_result_prunes: u64,
    tool_outcome_prunes: u64,
}

fn aggregate_window(samples: &[CompactionHygieneSample]) -> RuntimeSnapshotCompactionHygieneWindow {
    let mut window = RuntimeSnapshotCompactionHygieneWindow {
        sampled_session_count: samples.len(),
        ..RuntimeSnapshotCompactionHygieneWindow::default()
    };

    for sample in samples {
        if sample.read_error {
            window.sampled_session_read_errors =
                window.sampled_session_read_errors.saturating_add(1);
        }
        if sample.has_diagnostics {
            window.sessions_with_diagnostics = window.sessions_with_diagnostics.saturating_add(1);
        }
        if sample.failed_open {
            window.failed_open_session_count = window.failed_open_session_count.saturating_add(1);
        }
        window.total_demoted_recent_turns = window
            .total_demoted_recent_turns
            .saturating_add(sample.demoted_recent_turns);
        window.total_low_signal_turns = window
            .total_low_signal_turns
            .saturating_add(sample.low_signal_turns);
        window.total_tool_result_prunes = window
            .total_tool_result_prunes
            .saturating_add(sample.tool_result_prunes);
        window.total_tool_outcome_prunes = window
            .total_tool_outcome_prunes
            .saturating_add(sample.tool_outcome_prunes);
    }

    window
}

fn milli_ratio(numerator: u64, denominator: u64) -> Option<u32> {
    if denominator == 0 {
        return None;
    }
    Some(((numerator.saturating_mul(1000)) / denominator) as u32)
}

#[derive(Debug, Clone, Copy)]
enum TrendDirectionMode {
    HigherIsBetter,
    HigherIsWorse,
}

fn compare_trend_milli(
    recent: Option<u32>,
    baseline: Option<u32>,
    mode: TrendDirectionMode,
) -> &'static str {
    let (Some(recent), Some(baseline)) = (recent, baseline) else {
        return "insufficient_history";
    };

    let recent = recent as i64;
    let baseline = baseline as i64;
    let delta = recent - baseline;
    if delta.abs() <= i64::from(COMPACTION_HYGIENE_TREND_TOLERANCE_MILLI) {
        return "steady";
    }

    match mode {
        TrendDirectionMode::HigherIsBetter => {
            if delta > 0 {
                "improving"
            } else {
                "worsening"
            }
        }
        TrendDirectionMode::HigherIsWorse => {
            if delta > 0 {
                "worsening"
            } else {
                "improving"
            }
        }
    }
}

fn format_ratio_rollup(numerator: u64, denominator: u64, milli: Option<u32>) -> String {
    format!(
        "{numerator}/{denominator} ({})",
        format_compaction_hygiene_percent(milli)
    )
}

fn format_pressure_rollup(milli: Option<u32>) -> String {
    milli
        .map(|raw| format!("{:.3}/session", (raw as f64) / 1000.0))
        .unwrap_or_else(|| "-".to_owned())
}

fn format_compaction_hygiene_percent(milli: Option<u32>) -> String {
    milli
        .map(|raw| format!("{:.1}%", (raw as f64) / 10.0))
        .unwrap_or_else(|| "-".to_owned())
}

fn resolve_compaction_hygiene_evidence_status(
    sampled_session_count: usize,
    sessions_with_diagnostics: usize,
    sampled_session_read_errors: usize,
) -> &'static str {
    if sampled_session_count == 0 {
        "idle"
    } else if sampled_session_read_errors == sampled_session_count {
        "read_error"
    } else if sessions_with_diagnostics == 0 {
        if sampled_session_read_errors > 0 {
            "partial"
        } else {
            "no_evidence"
        }
    } else if sampled_session_read_errors > 0 {
        "partial"
    } else {
        "ok"
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeSnapshotCompactionHygieneState, RuntimeSnapshotCompactionHygieneWindow,
        RuntimeSnapshotCompactionLineageState, resolve_compaction_hygiene_evidence_status,
    };
    use crate::mvp::conversation::{TurnCheckpointProgressStatus, TurnCheckpointRecoveryAction};
    use crate::runtime_snapshot_compaction_sequence::RuntimeSnapshotCheckpointRepairManualReason;

    pub(crate) fn primary_lineage_retryable_state() -> RuntimeSnapshotCompactionLineageState {
        RuntimeSnapshotCompactionLineageState {
            root_session_id: Some("root-session".to_owned()),
            sampled_session_count: 2,
            compaction_sample_count: 2,
            latest_compaction_status: Some(TurnCheckpointProgressStatus::FailedOpen),
            compaction_failure_streak: 1,
            checkpoint_event_count: 2,
            checkpoint_failure_streak: 1,
            checkpoint_repair_action: Some(TurnCheckpointRecoveryAction::RunCompaction),
            checkpoint_repair_manual_reason: None,
        }
    }

    pub(crate) fn sample_retry_exhausted_compaction_hygiene_state()
    -> RuntimeSnapshotCompactionHygieneState {
        RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "ok".to_owned(),
            trend_scope: "primary_lineage".to_owned(),
            primary_lineage: RuntimeSnapshotCompactionLineageState {
                checkpoint_failure_streak: 2,
                ..primary_lineage_retryable_state()
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
    fn resolve_compaction_hygiene_evidence_status_distinguishes_idle_missing_partial_and_errors() {
        assert_eq!(resolve_compaction_hygiene_evidence_status(0, 0, 0), "idle");
        assert_eq!(
            resolve_compaction_hygiene_evidence_status(4, 0, 0),
            "no_evidence"
        );
        assert_eq!(
            resolve_compaction_hygiene_evidence_status(4, 0, 1),
            "partial"
        );
        assert_eq!(resolve_compaction_hygiene_evidence_status(4, 1, 0), "ok");
        assert_eq!(
            resolve_compaction_hygiene_evidence_status(4, 1, 1),
            "partial"
        );
        assert_eq!(
            resolve_compaction_hygiene_evidence_status(4, 0, 4),
            "read_error"
        );
    }

    #[test]
    fn decode_or_unknown_falls_back_for_missing_or_invalid_payloads() {
        let unknown = RuntimeSnapshotCompactionHygieneState::unknown_unavailable();

        assert_eq!(
            RuntimeSnapshotCompactionHygieneState::decode_or_unknown(None),
            unknown
        );

        let invalid = serde_json::json!({
            "strategy": "broken"
        });
        assert_eq!(
            RuntimeSnapshotCompactionHygieneState::decode_or_unknown(Some(&invalid)),
            unknown
        );
    }

    #[test]
    fn compaction_hygiene_posture_prioritizes_unavailable_idle_and_failed_open_pressure() {
        let mut state = RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "unavailable".to_owned(),
            trend_scope: "unavailable".to_owned(),
            primary_lineage: RuntimeSnapshotCompactionLineageState::default(),
            overall_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            recent_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            baseline_window: RuntimeSnapshotCompactionHygieneWindow::default(),
            error: Some("boom".to_owned()),
        };
        assert_eq!(state.posture(), "unavailable");

        state.evidence_status = "idle".to_owned();
        state.error = None;
        state.trend_scope = "idle".to_owned();
        assert_eq!(state.posture(), "idle");

        state.evidence_status = "ok".to_owned();
        state.overall_window.sampled_session_count = 4;
        state.overall_window.sessions_with_diagnostics = 2;
        state.overall_window.failed_open_session_count = 1;
        assert_eq!(state.posture(), "degraded");

        state.overall_window.failed_open_session_count = 0;
        assert_eq!(state.posture(), "healthy");

        state.evidence_status = "partial".to_owned();
        assert_eq!(state.posture(), "attention");
    }

    #[test]
    fn compaction_hygiene_trends_compare_recent_window_against_baseline_window() {
        let state = sample_retry_exhausted_compaction_hygiene_state();

        assert_eq!(state.reliability_trend(), "worsening");
        assert_eq!(state.coverage_trend(), "steady");
        assert_eq!(state.pressure_trend(), "steady");
        assert_eq!(state.continuity_health(), "broken");
        assert_eq!(state.continuity_repairability(), "retryable");
        assert_eq!(state.recovery_posture(), "retry_exhausted");
        assert_eq!(
            state.trend_summary(),
            "scope=primary_lineage reliability=worsening coverage=steady pressure=steady"
        );
    }

    #[test]
    fn trend_scope_defaults_to_recent_sessions_fallback_when_primary_lineage_is_too_small() {
        let state = RuntimeSnapshotCompactionHygieneState {
            strategy: "turn_floor_only".to_owned(),
            diagnostics_surface: "turn_checkpoint".to_owned(),
            evidence_status: "ok".to_owned(),
            trend_scope: "recent_sessions_fallback".to_owned(),
            primary_lineage: RuntimeSnapshotCompactionLineageState {
                root_session_id: Some("root-session".to_owned()),
                sampled_session_count: 1,
                compaction_sample_count: 1,
                latest_compaction_status: Some(TurnCheckpointProgressStatus::Completed),
                compaction_failure_streak: 0,
                checkpoint_event_count: 1,
                checkpoint_failure_streak: 0,
                checkpoint_repair_action: Some(TurnCheckpointRecoveryAction::None),
                checkpoint_repair_manual_reason: None,
            },
            overall_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 4,
                sessions_with_diagnostics: 2,
                sampled_session_read_errors: 0,
                failed_open_session_count: 0,
                total_demoted_recent_turns: 3,
                total_low_signal_turns: 2,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 1,
            },
            recent_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 2,
                sessions_with_diagnostics: 1,
                sampled_session_read_errors: 0,
                failed_open_session_count: 0,
                total_demoted_recent_turns: 2,
                total_low_signal_turns: 1,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 0,
            },
            baseline_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 2,
                sessions_with_diagnostics: 1,
                sampled_session_read_errors: 0,
                failed_open_session_count: 0,
                total_demoted_recent_turns: 1,
                total_low_signal_turns: 1,
                total_tool_result_prunes: 0,
                total_tool_outcome_prunes: 1,
            },
            error: None,
        };

        assert_eq!(state.trend_scope(), "recent_sessions_fallback");
        assert_eq!(state.continuity_health(), "scope_limited");
        assert_eq!(state.continuity_repairability(), "scope_limited");
        assert_eq!(state.recovery_posture(), "scope_limited");
        assert_eq!(
            state.trend_summary(),
            "scope=recent_sessions_fallback reliability=steady coverage=steady pressure=worsening"
        );
    }

    #[test]
    fn continuity_health_marks_primary_lineage_failure_streak_as_broken() {
        let state = RuntimeSnapshotCompactionHygieneState {
            primary_lineage: RuntimeSnapshotCompactionLineageState {
                sampled_session_count: 3,
                compaction_sample_count: 3,
                checkpoint_failure_streak: 2,
                checkpoint_repair_action: Some(TurnCheckpointRecoveryAction::InspectManually),
                checkpoint_repair_manual_reason: Some(
                    RuntimeSnapshotCheckpointRepairManualReason::CheckpointStateRequiresManualInspection,
                ),
                ..primary_lineage_retryable_state()
            },
            overall_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 5,
                sessions_with_diagnostics: 3,
                sampled_session_read_errors: 0,
                failed_open_session_count: 2,
                total_demoted_recent_turns: 4,
                total_low_signal_turns: 3,
                total_tool_result_prunes: 2,
                total_tool_outcome_prunes: 1,
            },
            recent_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 2,
                sessions_with_diagnostics: 2,
                sampled_session_read_errors: 0,
                failed_open_session_count: 2,
                total_demoted_recent_turns: 3,
                total_low_signal_turns: 2,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 1,
            },
            baseline_window: RuntimeSnapshotCompactionHygieneWindow {
                sampled_session_count: 1,
                sessions_with_diagnostics: 1,
                sampled_session_read_errors: 0,
                failed_open_session_count: 0,
                total_demoted_recent_turns: 1,
                total_low_signal_turns: 1,
                total_tool_result_prunes: 1,
                total_tool_outcome_prunes: 0,
            },
            ..sample_retry_exhausted_compaction_hygiene_state()
        };

        assert_eq!(state.continuity_health(), "broken");
        assert_eq!(state.continuity_repairability(), "manual_inspection");
        assert_eq!(state.recovery_posture(), "manual_lane");
        let assessment = state.assessment();
        assert_eq!(assessment.recovery_posture.as_str(), "manual_lane");
        assert_eq!(assessment.continuity_health.as_str(), "broken");
        assert_eq!(
            assessment.continuity_repairability.as_str(),
            "manual_inspection"
        );
    }

    #[test]
    fn compaction_hygiene_state_deserializes_flat_overall_window_fields() {
        let value = serde_json::json!({
            "strategy": "unknown",
            "diagnostics_surface": "turn_checkpoint",
            "evidence_status": "unavailable",
            "trend_scope": "unavailable",
            "primary_lineage": {
                "root_session_id": null,
                "sampled_session_count": 0,
                "compaction_sample_count": 0,
                "latest_compaction_status": null,
                "compaction_failure_streak": 0,
                "checkpoint_event_count": 0,
                "checkpoint_failure_streak": 0,
                "checkpoint_repair_action": null,
                "checkpoint_repair_manual_reason": null
            },
            "sampled_session_count": 4,
            "sessions_with_diagnostics": 2,
            "sampled_session_read_errors": 1,
            "failed_open_session_count": 1,
            "total_demoted_recent_turns": 3,
            "total_low_signal_turns": 4,
            "total_tool_result_prunes": 2,
            "total_tool_outcome_prunes": 1,
            "recent_window": {
                "sampled_session_count": 2,
                "sessions_with_diagnostics": 1,
                "sampled_session_read_errors": 0,
                "failed_open_session_count": 1,
                "total_demoted_recent_turns": 2,
                "total_low_signal_turns": 2,
                "total_tool_result_prunes": 1,
                "total_tool_outcome_prunes": 0
            },
            "baseline_window": {
                "sampled_session_count": 2,
                "sessions_with_diagnostics": 1,
                "sampled_session_read_errors": 1,
                "failed_open_session_count": 0,
                "total_demoted_recent_turns": 1,
                "total_low_signal_turns": 2,
                "total_tool_result_prunes": 1,
                "total_tool_outcome_prunes": 1
            },
            "error": null
        });

        let state: RuntimeSnapshotCompactionHygieneState =
            serde_json::from_value(value).expect("deserialize compaction hygiene state");

        assert_eq!(state.overall_window.sampled_session_count, 4);
        assert_eq!(state.overall_window.sessions_with_diagnostics, 2);
        assert_eq!(state.overall_window.sampled_session_read_errors, 1);
        assert_eq!(state.overall_window.failed_open_session_count, 1);
        assert_eq!(state.overall_window.total_demoted_recent_turns, 3);
        assert_eq!(state.overall_window.total_low_signal_turns, 4);
    }
}
