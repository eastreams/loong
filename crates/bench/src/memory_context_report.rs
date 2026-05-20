use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::resolve_memory_context_benchmark_temp_root;
use super::super::{
    CliResult, NumericStats, compute_numeric_stats, current_epoch_seconds, write_json_file,
};
use super::super::Value;

const DEFAULT_MEMORY_CONTEXT_MIN_SPEEDUP_RATIO: f64 = 1.2;
const DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_RATIO_P95: f64 = 1.15;
const DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_OVERHEAD_P95_MS: f64 = 0.050;
const DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_WARNING_MIN_SAMPLES: usize = 8;
const DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_NOISY_SUPPRESSION_MAX_RATIO_P95: f64 = 1.20;
const DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_NOISY_SUPPRESSION_MAX_OVERHEAD_P95_MS: f64 = 0.150;
const DEFAULT_MEMORY_CONTEXT_REBUILD_BUDGET_CHANGE_SOFT_MAX_RATIO_P95: f64 = 1.05;
const DEFAULT_MEMORY_CONTEXT_METADATA_REALIGN_SOFT_MAX_RATIO_P95: f64 = 1.10;
const DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_WARNING_MIN_SUITES: usize = 3;
const DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50: f64 = 0.75;
const DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_CLEAR_WIN_SUPPRESSION_MULTIPLIER: f64 = 1.5;
const DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_TINY_HOT_PATH_MAX_P50_MS: f64 = 1.0;
const DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_TINY_HOT_PATH_MAX_RANGE_MS: f64 = 1.25;
const MEMORY_CONTEXT_SUITE_AGGREGATION_MEDIAN_OF_P95: &str = "median_of_suite_p95";

#[derive(Debug, Clone, Serialize)]
struct MemoryContextBenchmarkReport {
    generated_at_epoch_s: u64,
    profile: String,
    output_path: String,
    benchmark_temp_root: String,
    benchmark_temp_root_source: MemoryContextBenchmarkTempRootSource,
    suite_repetitions: usize,
    suite_aggregation: String,
    rss_telemetry_scope: String,
    history_turns: usize,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    seed_db_bytes: u64,
    suite_p95_summaries: Vec<MemoryContextSuiteP95Summary>,
    suite_stability: MemoryContextSuiteStabilitySummary,
    cold_path_phases: MemoryContextColdPathPhaseReport,
    cold_path_phase_stability: MemoryContextColdPathPhaseStabilityReport,
    cold_path_noise_attribution: MemoryContextColdPathNoiseAttributionReport,
    cold_path_bootstrap_noise_attribution: MemoryContextColdPathBootstrapNoiseAttributionReport,
    cold_path_load_noise_attribution: MemoryContextColdPathLoadNoiseAttributionReport,
    window_only_latency_ms: NumericStats,
    summary_window_cover_latency_ms: NumericStats,
    summary_rebuild_latency_ms: NumericStats,
    summary_rebuild_budget_change_latency_ms: NumericStats,
    summary_metadata_realign_latency_ms: NumericStats,
    summary_steady_state_latency_ms: NumericStats,
    window_shrink_catch_up_latency_ms: NumericStats,
    window_only_append_pre_overflow_latency_ms: NumericStats,
    window_only_append_cold_overflow_latency_ms: NumericStats,
    summary_append_pre_overflow_latency_ms: NumericStats,
    summary_append_cold_overflow_latency_ms: NumericStats,
    summary_append_saturated_latency_ms: NumericStats,
    window_only_rss_delta_kib: NumericStats,
    summary_window_cover_rss_delta_kib: NumericStats,
    summary_rebuild_rss_delta_kib: NumericStats,
    summary_rebuild_budget_change_rss_delta_kib: NumericStats,
    summary_metadata_realign_rss_delta_kib: NumericStats,
    summary_steady_state_rss_delta_kib: NumericStats,
    window_shrink_catch_up_rss_delta_kib: NumericStats,
    window_only_append_pre_overflow_rss_delta_kib: NumericStats,
    window_only_append_cold_overflow_rss_delta_kib: NumericStats,
    summary_append_pre_overflow_rss_delta_kib: NumericStats,
    summary_append_cold_overflow_rss_delta_kib: NumericStats,
    summary_append_saturated_rss_delta_kib: NumericStats,
    window_only_entry_count: usize,
    window_only_turn_entries: usize,
    window_only_payload_chars: usize,
    summary_window_cover_entry_count: usize,
    summary_window_cover_turn_entries: usize,
    summary_window_cover_payload_chars: usize,
    summary_rebuild_entry_count: usize,
    summary_rebuild_turn_entries: usize,
    summary_rebuild_summary_chars: usize,
    summary_rebuild_payload_chars: usize,
    summary_rebuild_budget_change_entry_count: usize,
    summary_rebuild_budget_change_turn_entries: usize,
    summary_rebuild_budget_change_summary_chars: usize,
    summary_rebuild_budget_change_payload_chars: usize,
    summary_metadata_realign_entry_count: usize,
    summary_metadata_realign_turn_entries: usize,
    summary_metadata_realign_summary_chars: usize,
    summary_metadata_realign_payload_chars: usize,
    summary_steady_state_entry_count: usize,
    summary_steady_state_turn_entries: usize,
    summary_steady_state_summary_chars: usize,
    summary_steady_state_payload_chars: usize,
    window_shrink_catch_up_entry_count: usize,
    window_shrink_catch_up_turn_entries: usize,
    window_shrink_catch_up_summary_chars: usize,
    window_shrink_catch_up_payload_chars: usize,
    prompt_efficiency_signals: MemoryContextPromptEfficiencySignals,
    flattened_sample_ratios: MemoryContextRatioP95Summary,
    aggregated_p95_median_ms: MemoryContextAggregatedP95MedianMs,
    aggregated_ratios: MemoryContextRatioP95Summary,
    gate: MemoryContextBenchmarkGateSummary,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextPromptEfficiencySignals {
    window_only: MemoryContextPromptEfficiencySignal,
    summary_window_cover: MemoryContextPromptEfficiencySignal,
    summary_rebuild: MemoryContextPromptEfficiencySignal,
    summary_rebuild_budget_change: MemoryContextPromptEfficiencySignal,
    summary_metadata_realign: MemoryContextPromptEfficiencySignal,
    summary_steady_state: MemoryContextPromptEfficiencySignal,
    window_shrink_catch_up: MemoryContextPromptEfficiencySignal,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextPromptEfficiencySignal {
    entry_count: usize,
    turn_entries: usize,
    estimated_session_local_recall_chars: usize,
    estimated_non_recall_context_chars: usize,
    estimated_session_local_recall_share_ratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MemoryContextBenchmarkTempRootSource {
    Explicit,
    CurrentExeTargetDir,
    OutputParent,
    SystemTemp,
}

impl MemoryContextBenchmarkTempRootSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::CurrentExeTargetDir => "current_exe_target_dir",
            Self::OutputParent => "output_parent",
            Self::SystemTemp => "system_temp",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedMemoryContextBenchmarkTempRoot {
    pub(super) path: PathBuf,
    pub(super) source: MemoryContextBenchmarkTempRootSource,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextBenchmarkGateSummary {
    enforced: bool,
    passed: bool,
    min_steady_state_speedup_ratio: f64,
    observed_speedup_ratio: Option<f64>,
    summary_window_cover_soft_max_ratio_p95: f64,
    summary_window_cover_soft_max_overhead_p95_ms: f64,
    summary_window_cover_soft_warning_min_samples: usize,
    summary_rebuild_budget_change_vs_rebuild_soft_max_ratio_p95: f64,
    summary_metadata_realign_vs_budget_change_soft_max_ratio_p95: f64,
    suite_stability_soft_warning_min_suites: usize,
    suite_stability_soft_max_range_over_p50: f64,
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextAggregatedP95MedianMs {
    window_only: Option<f64>,
    summary_window_cover: Option<f64>,
    summary_rebuild: Option<f64>,
    summary_rebuild_budget_change: Option<f64>,
    summary_metadata_realign: Option<f64>,
    summary_steady_state: Option<f64>,
    window_shrink_catch_up: Option<f64>,
    window_only_append_pre_overflow: Option<f64>,
    window_only_append_cold_overflow: Option<f64>,
    summary_append_pre_overflow: Option<f64>,
    summary_append_cold_overflow: Option<f64>,
    summary_append_saturated: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextRatioP95Summary {
    summary_window_cover_vs_window_only_ratio_p95: Option<f64>,
    summary_window_cover_overhead_p95_ms: Option<f64>,
    summary_rebuild_budget_change_vs_rebuild_ratio_p95: Option<f64>,
    summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95: Option<f64>,
    summary_metadata_realign_vs_budget_change_ratio_p95: Option<f64>,
    speedup_ratio_p95: Option<f64>,
    window_shrink_catch_up_vs_rebuild_speedup_ratio_p95: Option<f64>,
    summary_append_pre_overflow_vs_window_only_ratio_p95: Option<f64>,
    summary_append_cold_overflow_vs_window_only_ratio_p95: Option<f64>,
}

#[derive(Debug, Clone, Default)]
#[doc(hidden)]
pub struct MemoryContextColdPathPhaseSamples {
    pub copy_db_ms: Vec<f64>,
    pub source_bootstrap_ms: Vec<f64>,
    pub source_bootstrap_normalize_path_ms: Vec<f64>,
    pub source_bootstrap_registry_lock_ms: Vec<f64>,
    pub source_bootstrap_registry_lookup_ms: Vec<f64>,
    pub source_bootstrap_runtime_create_ms: Vec<f64>,
    pub source_bootstrap_parent_dir_create_ms: Vec<f64>,
    pub source_bootstrap_connection_open_ms: Vec<f64>,
    pub source_bootstrap_configure_connection_ms: Vec<f64>,
    pub source_bootstrap_schema_init_ms: Vec<f64>,
    pub source_bootstrap_schema_upgrade_ms: Vec<f64>,
    pub source_bootstrap_registry_insert_ms: Vec<f64>,
    pub source_warmup_ms: Vec<f64>,
    pub append_turn_ms: Vec<f64>,
    pub target_bootstrap_ms: Vec<f64>,
    pub target_bootstrap_normalize_path_ms: Vec<f64>,
    pub target_bootstrap_registry_lock_ms: Vec<f64>,
    pub target_bootstrap_registry_lookup_ms: Vec<f64>,
    pub target_bootstrap_runtime_create_ms: Vec<f64>,
    pub target_bootstrap_parent_dir_create_ms: Vec<f64>,
    pub target_bootstrap_connection_open_ms: Vec<f64>,
    pub target_bootstrap_configure_connection_ms: Vec<f64>,
    pub target_bootstrap_schema_init_ms: Vec<f64>,
    pub target_bootstrap_schema_upgrade_ms: Vec<f64>,
    pub target_bootstrap_registry_insert_ms: Vec<f64>,
    pub target_load_ms: Vec<f64>,
    pub target_load_window_query_ms: Vec<f64>,
    pub target_load_window_turn_count_query_ms: Vec<f64>,
    pub target_load_window_exact_rows_query_ms: Vec<f64>,
    pub target_load_window_known_overflow_rows_query_ms: Vec<f64>,
    pub target_load_window_fallback_rows_query_ms: Vec<f64>,
    pub target_load_summary_checkpoint_meta_query_ms: Vec<f64>,
    pub target_load_summary_checkpoint_body_load_ms: Vec<f64>,
    pub target_load_summary_checkpoint_metadata_update_ms: Vec<f64>,
    pub target_load_summary_checkpoint_metadata_update_returning_body_ms: Vec<f64>,
    pub target_load_summary_rebuild_ms: Vec<f64>,
    pub target_load_summary_rebuild_stream_ms: Vec<f64>,
    pub target_load_summary_rebuild_checkpoint_upsert_ms: Vec<f64>,
    pub target_load_summary_rebuild_checkpoint_metadata_upsert_ms: Vec<f64>,
    pub target_load_summary_rebuild_checkpoint_body_upsert_ms: Vec<f64>,
    pub target_load_summary_rebuild_checkpoint_commit_ms: Vec<f64>,
    pub target_load_summary_catch_up_ms: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct NumericSpreadSummary {
    count: usize,
    min: Option<f64>,
    p50: Option<f64>,
    max: Option<f64>,
    range: Option<f64>,
    range_over_p50: Option<f64>,
    max_over_p50: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathPhaseStats {
    copy_db_ms: NumericStats,
    source_bootstrap_ms: NumericStats,
    source_warmup_ms: NumericStats,
    append_turn_ms: NumericStats,
    target_bootstrap_ms: NumericStats,
    target_load_ms: NumericStats,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathPhaseReport {
    summary_rebuild: MemoryContextColdPathPhaseStats,
    summary_rebuild_budget_change: MemoryContextColdPathPhaseStats,
    summary_metadata_realign: MemoryContextColdPathPhaseStats,
    window_shrink_catch_up: MemoryContextColdPathPhaseStats,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextSuiteStabilitySummary {
    window_only_p95_ms: NumericSpreadSummary,
    summary_window_cover_p95_ms: NumericSpreadSummary,
    summary_rebuild_p95_ms: NumericSpreadSummary,
    summary_rebuild_budget_change_p95_ms: NumericSpreadSummary,
    summary_metadata_realign_p95_ms: NumericSpreadSummary,
    summary_steady_state_p95_ms: NumericSpreadSummary,
    window_shrink_catch_up_p95_ms: NumericSpreadSummary,
    window_only_append_pre_overflow_p95_ms: NumericSpreadSummary,
    window_only_append_cold_overflow_p95_ms: NumericSpreadSummary,
    summary_append_pre_overflow_p95_ms: NumericSpreadSummary,
    summary_append_cold_overflow_p95_ms: NumericSpreadSummary,
    summary_append_saturated_p95_ms: NumericSpreadSummary,
    summary_window_cover_vs_window_only_ratio_p95: NumericSpreadSummary,
    summary_window_cover_overhead_p95_ms: NumericSpreadSummary,
    summary_rebuild_budget_change_vs_rebuild_ratio_p95: NumericSpreadSummary,
    summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95: NumericSpreadSummary,
    summary_metadata_realign_vs_budget_change_ratio_p95: NumericSpreadSummary,
    speedup_ratio_p95: NumericSpreadSummary,
    window_shrink_catch_up_vs_rebuild_speedup_ratio_p95: NumericSpreadSummary,
    summary_append_pre_overflow_vs_window_only_ratio_p95: NumericSpreadSummary,
    summary_append_cold_overflow_vs_window_only_ratio_p95: NumericSpreadSummary,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathPhaseStabilitySummary {
    copy_db_ms: NumericSpreadSummary,
    source_bootstrap_ms: NumericSpreadSummary,
    source_warmup_ms: NumericSpreadSummary,
    append_turn_ms: NumericSpreadSummary,
    target_bootstrap_ms: NumericSpreadSummary,
    target_load_ms: NumericSpreadSummary,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathPhaseStabilityReport {
    summary_rebuild: MemoryContextColdPathPhaseStabilitySummary,
    summary_rebuild_budget_change: MemoryContextColdPathPhaseStabilitySummary,
    summary_metadata_realign: MemoryContextColdPathPhaseStabilitySummary,
    window_shrink_catch_up: MemoryContextColdPathPhaseStabilitySummary,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathNoiseAttribution {
    phase: String,
    range_over_p50: f64,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathNoiseAttributionReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_rebuild: Option<MemoryContextColdPathNoiseAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_rebuild_budget_change: Option<MemoryContextColdPathNoiseAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_metadata_realign: Option<MemoryContextColdPathNoiseAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_shrink_catch_up: Option<MemoryContextColdPathNoiseAttribution>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextBootstrapNoiseAttribution {
    phase: String,
    range_over_p50: f64,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathBootstrapNoiseAttribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    source_bootstrap: Option<MemoryContextBootstrapNoiseAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_bootstrap: Option<MemoryContextBootstrapNoiseAttribution>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathBootstrapNoiseAttributionReport {
    summary_rebuild: MemoryContextColdPathBootstrapNoiseAttribution,
    summary_rebuild_budget_change: MemoryContextColdPathBootstrapNoiseAttribution,
    summary_metadata_realign: MemoryContextColdPathBootstrapNoiseAttribution,
    window_shrink_catch_up: MemoryContextColdPathBootstrapNoiseAttribution,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextLoadNoiseAttribution {
    phase: String,
    range_over_p50: f64,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathLoadNoiseAttribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    target_load: Option<MemoryContextLoadNoiseAttribution>,
}

#[derive(Debug, Clone, Serialize)]
struct MemoryContextColdPathLoadNoiseAttributionReport {
    summary_rebuild: MemoryContextColdPathLoadNoiseAttribution,
    summary_rebuild_budget_change: MemoryContextColdPathLoadNoiseAttribution,
    summary_metadata_realign: MemoryContextColdPathLoadNoiseAttribution,
    window_shrink_catch_up: MemoryContextColdPathLoadNoiseAttribution,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProgrammaticPressureScenarioGate {
    pub(crate) passed: bool,
    pub(crate) checks: Vec<ProgrammaticPressureGateCheck>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProgrammaticPressureGateCheck {
    pub(crate) metric: String,
    pub(crate) comparator: String,
    pub(crate) threshold: f64,
    pub(crate) observed: f64,
    pub(crate) passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) baseline_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[doc(hidden)]
pub(crate) struct ScenarioRunSample {
    pub latency_ms: f64,
    pub passed: bool,
    pub blocked: bool,
    pub connector_calls: usize,
    pub error_codes: BTreeMap<String, usize>,
    pub schema_fingerprint: Option<String>,
    pub(crate) scheduler: Option<SchedulerSnapshot>,
    pub half_open_transition_ms: Option<f64>,
    pub closed_after_recovery: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) peak_in_flight: usize,
    pub(crate) final_in_flight_budget: usize,
    pub(crate) budget_reductions: usize,
    pub(crate) budget_increases: usize,
    pub(crate) wait_cycles: usize,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct MemoryContextBenchmarkSuiteSamples {
    pub seed_db_bytes: u64,
    pub window_only_samples: Vec<f64>,
    pub summary_window_cover_samples: Vec<f64>,
    pub summary_rebuild_samples: Vec<f64>,
    pub summary_rebuild_budget_change_samples: Vec<f64>,
    pub summary_metadata_realign_samples: Vec<f64>,
    pub summary_steady_state_samples: Vec<f64>,
    pub window_shrink_catch_up_samples: Vec<f64>,
    pub window_only_append_pre_overflow_samples: Vec<f64>,
    pub window_only_append_cold_overflow_samples: Vec<f64>,
    pub summary_append_pre_overflow_samples: Vec<f64>,
    pub summary_append_cold_overflow_samples: Vec<f64>,
    pub summary_append_saturated_samples: Vec<f64>,
    pub window_only_rss_deltas_kib: Vec<f64>,
    pub summary_window_cover_rss_deltas_kib: Vec<f64>,
    pub summary_rebuild_rss_deltas_kib: Vec<f64>,
    pub summary_rebuild_budget_change_rss_deltas_kib: Vec<f64>,
    pub summary_metadata_realign_rss_deltas_kib: Vec<f64>,
    pub summary_steady_state_rss_deltas_kib: Vec<f64>,
    pub window_shrink_catch_up_rss_deltas_kib: Vec<f64>,
    pub window_only_append_pre_overflow_rss_deltas_kib: Vec<f64>,
    pub window_only_append_cold_overflow_rss_deltas_kib: Vec<f64>,
    pub summary_append_pre_overflow_rss_deltas_kib: Vec<f64>,
    pub summary_append_cold_overflow_rss_deltas_kib: Vec<f64>,
    pub summary_append_saturated_rss_deltas_kib: Vec<f64>,
    pub summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples,
    pub summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples,
    pub summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples,
    pub window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples,
    pub window_only_shape: MemoryContextShape,
    pub summary_window_cover_shape: MemoryContextShape,
    pub summary_rebuild_shape: MemoryContextShape,
    pub summary_rebuild_budget_change_shape: MemoryContextShape,
    pub summary_metadata_realign_shape: MemoryContextShape,
    pub summary_steady_state_shape: MemoryContextShape,
    pub window_shrink_catch_up_shape: MemoryContextShape,
}

#[derive(Debug, Clone, Default, Serialize)]
struct MemoryContextSuiteP95Summary {
    window_only: Option<f64>,
    summary_window_cover: Option<f64>,
    summary_rebuild: Option<f64>,
    summary_rebuild_budget_change: Option<f64>,
    summary_metadata_realign: Option<f64>,
    summary_steady_state: Option<f64>,
    window_shrink_catch_up: Option<f64>,
    window_only_append_pre_overflow: Option<f64>,
    window_only_append_cold_overflow: Option<f64>,
    summary_append_pre_overflow: Option<f64>,
    summary_append_cold_overflow: Option<f64>,
    summary_append_saturated: Option<f64>,
    summary_window_cover_vs_window_only_ratio_p95: Option<f64>,
    summary_window_cover_overhead_p95_ms: Option<f64>,
    summary_rebuild_budget_change_vs_rebuild_ratio_p95: Option<f64>,
    summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95: Option<f64>,
    summary_metadata_realign_vs_budget_change_ratio_p95: Option<f64>,
    speedup_ratio_p95: Option<f64>,
    window_shrink_catch_up_vs_rebuild_speedup_ratio_p95: Option<f64>,
    summary_append_pre_overflow_vs_window_only_ratio_p95: Option<f64>,
    summary_append_cold_overflow_vs_window_only_ratio_p95: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
#[doc(hidden)]
pub struct MemoryContextShape {
    pub entry_count: usize,
    pub turn_entries: usize,
    pub summary_chars: usize,
    pub payload_chars: usize,
}

fn memory_context_prompt_efficiency_signal(
    shape: MemoryContextShape,
) -> MemoryContextPromptEfficiencySignal {
    let estimated_session_local_recall_chars = shape.summary_chars;
    let estimated_non_recall_context_chars = shape
        .payload_chars
        .saturating_sub(estimated_session_local_recall_chars);
    let estimated_session_local_recall_share_ratio =
        compute_ratio_f64(estimated_session_local_recall_chars, shape.payload_chars);

    MemoryContextPromptEfficiencySignal {
        entry_count: shape.entry_count,
        turn_entries: shape.turn_entries,
        estimated_session_local_recall_chars,
        estimated_non_recall_context_chars,
        estimated_session_local_recall_share_ratio,
    }
}

fn build_memory_context_prompt_efficiency_signals(
    representative: &MemoryContextBenchmarkSuiteSamples,
) -> MemoryContextPromptEfficiencySignals {
    let window_only = memory_context_prompt_efficiency_signal(representative.window_only_shape);
    let summary_window_cover =
        memory_context_prompt_efficiency_signal(representative.summary_window_cover_shape);
    let summary_rebuild =
        memory_context_prompt_efficiency_signal(representative.summary_rebuild_shape);
    let summary_rebuild_budget_change =
        memory_context_prompt_efficiency_signal(representative.summary_rebuild_budget_change_shape);
    let summary_metadata_realign =
        memory_context_prompt_efficiency_signal(representative.summary_metadata_realign_shape);
    let summary_steady_state =
        memory_context_prompt_efficiency_signal(representative.summary_steady_state_shape);
    let window_shrink_catch_up =
        memory_context_prompt_efficiency_signal(representative.window_shrink_catch_up_shape);

    MemoryContextPromptEfficiencySignals {
        window_only,
        summary_window_cover,
        summary_rebuild,
        summary_rebuild_budget_change,
        summary_metadata_realign,
        summary_steady_state,
        window_shrink_catch_up,
    }
}

#[doc(hidden)]
pub type MemoryContextBenchmarkSuiteRunner = fn(
    temp_root_override: Option<&Path>,
    history_turns: usize,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
) -> CliResult<MemoryContextBenchmarkSuiteSamples>;

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct MemoryContextBenchmarkReportAugmentContext {
    pub output_path: String,
    pub benchmark_temp_root: PathBuf,
    pub history_turns: usize,
    pub sliding_window: usize,
    pub window_shrink_source_window: usize,
    pub summary_max_chars: usize,
    pub words_per_turn: usize,
    pub rebuild_iterations: usize,
    pub hot_iterations: usize,
    pub warmup_iterations: usize,
    pub suite_repetitions: usize,
    pub enforce_gate: bool,
    pub min_steady_state_speedup_ratio: f64,
}

#[doc(hidden)]
pub type MemoryContextBenchmarkReportAugmenter = fn(
    report: &mut Value,
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
    context: &MemoryContextBenchmarkReportAugmentContext,
) -> CliResult<()>;

pub fn run_memory_context_benchmark_cli_with_suite_runner(
    output_path: &str,
    temp_root: Option<&str>,
    history_turns: usize,
    sliding_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    suite_repetitions: usize,
    enforce_gate: bool,
    min_steady_state_speedup_ratio: f64,
    suite_runner: MemoryContextBenchmarkSuiteRunner,
    report_augmenter: Option<MemoryContextBenchmarkReportAugmenter>,
) -> CliResult<()> {
    if history_turns <= sliding_window {
        return Err("history_turns must exceed sliding_window to exercise summary mode".to_owned());
    }
    if history_turns <= sliding_window.saturating_add(1) {
        return Err(
            "history_turns must exceed sliding_window by at least 2 to exercise shrink catch-up mode"
                .to_owned(),
        );
    }
    if sliding_window == 0 {
        return Err("sliding_window must be >= 1".to_owned());
    }
    if summary_max_chars == 0 {
        return Err("summary_max_chars must be >= 1".to_owned());
    }
    if words_per_turn == 0 {
        return Err("words_per_turn must be >= 1".to_owned());
    }
    if rebuild_iterations == 0 {
        return Err("rebuild_iterations must be >= 1".to_owned());
    }
    if hot_iterations == 0 {
        return Err("hot_iterations must be >= 1".to_owned());
    }
    if suite_repetitions == 0 {
        return Err("suite_repetitions must be >= 1".to_owned());
    }

    let normalized_min_speedup_ratio =
        if min_steady_state_speedup_ratio.is_finite() && min_steady_state_speedup_ratio > 0.0 {
            min_steady_state_speedup_ratio
        } else {
            DEFAULT_MEMORY_CONTEXT_MIN_SPEEDUP_RATIO
        };
    let window_shrink_source_window =
        memory_context_window_shrink_source_window(history_turns, sliding_window)?;
    let temp_root = resolve_memory_context_benchmark_temp_root(output_path, temp_root)?;
    let mut suite_runs = Vec::with_capacity(suite_repetitions);
    for _ in 0..suite_repetitions {
        suite_runs.push(suite_runner(
            Some(temp_root.path.as_path()),
            history_turns,
            sliding_window,
            window_shrink_source_window,
            summary_max_chars,
            words_per_turn,
            rebuild_iterations,
            hot_iterations,
            warmup_iterations,
        )?);
    }
    let report = try_build_memory_context_benchmark_report(
        output_path,
        &temp_root,
        history_turns,
        sliding_window,
        window_shrink_source_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        &suite_runs,
        suite_repetitions,
        enforce_gate,
        normalized_min_speedup_ratio,
    )?;
    let report_augment_context = MemoryContextBenchmarkReportAugmentContext {
        output_path: output_path.to_owned(),
        benchmark_temp_root: temp_root.path.clone(),
        history_turns,
        sliding_window,
        window_shrink_source_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        suite_repetitions,
        enforce_gate,
        min_steady_state_speedup_ratio: normalized_min_speedup_ratio,
    };
    if let Some(report_augmenter) = report_augmenter {
        let mut report_value = serde_json::to_value(&report).map_err(|error| {
            format!("failed to serialize memory context benchmark report: {error}")
        })?;

        report_augmenter(&mut report_value, &suite_runs, &report_augment_context)?;
        write_json_file(output_path, &report_value)?;
    } else {
        write_json_file(output_path, &report)?;
    }
    println!("memory context benchmark report written to {output_path}");
    println!(
        "benchmark_temp_root={} source={}",
        temp_root.path.display(),
        temp_root.source.as_str()
    );
    println!(
        "suite_repetitions={} suite_aggregation={}",
        report.suite_repetitions, report.suite_aggregation
    );
    println!(
        "window_only p95={:.3}ms summary_window_cover p95={:.3}ms cover_vs_window_ratio_p95={:.3} cover_overhead_p95_ms={:.3} summary_rebuild p95={:.3}ms summary_rebuild_budget_change p95={:.3}ms budget_change_vs_rebuild_ratio_p95={:.3} budget_change_vs_rebuild_summary_char_adjusted_ratio_p95={:.3} summary_metadata_realign p95={:.3}ms metadata_realign_vs_budget_change_ratio_p95={:.3} summary_steady_state p95={:.3}ms window_shrink_catch_up p95={:.3}ms window_only_append_pre_overflow p95={:.3}ms summary_append_pre_overflow p95={:.3}ms append_pre_vs_window_only_ratio_p95={:.3} window_only_append_cold_overflow p95={:.3}ms summary_append_cold_overflow p95={:.3}ms append_cold_vs_window_only_ratio_p95={:.3} summary_append_saturated p95={:.3}ms speedup_ratio_p95={:.3} shrink_vs_rebuild_speedup_ratio_p95={:.3} gate={}",
        report.aggregated_p95_median_ms.window_only.unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_window_cover
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_window_cover_vs_window_only_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_window_cover_overhead_p95_ms
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_rebuild
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_rebuild_budget_change
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_rebuild_budget_change_vs_rebuild_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_metadata_realign
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_metadata_realign_vs_budget_change_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_steady_state
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .window_shrink_catch_up
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .window_only_append_pre_overflow
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_append_pre_overflow
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_append_pre_overflow_vs_window_only_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .window_only_append_cold_overflow
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_append_cold_overflow
            .unwrap_or(0.0),
        report
            .aggregated_ratios
            .summary_append_cold_overflow_vs_window_only_ratio_p95
            .unwrap_or(0.0),
        report
            .aggregated_p95_median_ms
            .summary_append_saturated
            .unwrap_or(0.0),
        report.aggregated_ratios.speedup_ratio_p95.unwrap_or(0.0),
        report
            .aggregated_ratios
            .window_shrink_catch_up_vs_rebuild_speedup_ratio_p95
            .unwrap_or(0.0),
        if report.gate.passed { "pass" } else { "fail" }
    );
    if report.suite_repetitions > 1 {
        println!(
            "flattened_sample_ratio_p95 cover_vs_window_ratio_p95={:.3} cover_overhead_p95_ms={:.3} budget_change_vs_rebuild_ratio_p95={:.3} budget_change_vs_rebuild_summary_char_adjusted_ratio_p95={:.3} metadata_realign_vs_budget_change_ratio_p95={:.3} append_pre_vs_window_only_ratio_p95={:.3} append_cold_vs_window_only_ratio_p95={:.3} speedup_ratio_p95={:.3} shrink_vs_rebuild_speedup_ratio_p95={:.3}",
            report
                .flattened_sample_ratios
                .summary_window_cover_vs_window_only_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_window_cover_overhead_p95_ms
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_rebuild_budget_change_vs_rebuild_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_metadata_realign_vs_budget_change_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_append_pre_overflow_vs_window_only_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .summary_append_cold_overflow_vs_window_only_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .speedup_ratio_p95
                .unwrap_or(0.0),
            report
                .flattened_sample_ratios
                .window_shrink_catch_up_vs_rebuild_speedup_ratio_p95
                .unwrap_or(0.0),
        );
        println!(
            "suite_stability_range_ms window_only={} summary_window_cover={} summary_rebuild={} summary_rebuild_budget_change={} summary_metadata_realign={} summary_steady_state={} window_shrink_catch_up={} suite_stability_range_over_p50(speedup/shrink_vs_rebuild)={}/{}",
            format_optional_decimal(report.suite_stability.window_only_p95_ms.range, 3),
            format_optional_decimal(report.suite_stability.summary_window_cover_p95_ms.range, 3),
            format_optional_decimal(report.suite_stability.summary_rebuild_p95_ms.range, 3),
            format_optional_decimal(
                report
                    .suite_stability
                    .summary_rebuild_budget_change_p95_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report.suite_stability.summary_metadata_realign_p95_ms.range,
                3
            ),
            format_optional_decimal(report.suite_stability.summary_steady_state_p95_ms.range, 3),
            format_optional_decimal(
                report.suite_stability.window_shrink_catch_up_p95_ms.range,
                3
            ),
            format_optional_decimal(report.suite_stability.speedup_ratio_p95.range_over_p50, 3),
            format_optional_decimal(
                report
                    .suite_stability
                    .window_shrink_catch_up_vs_rebuild_speedup_ratio_p95
                    .range_over_p50,
                3
            ),
        );
        println!(
            "cold_path_phase_range_ms rebuild(copy/target_bootstrap/target_load)={}/{}/{} budget_change(source_warmup/target_load)={}/{} metadata_realign(append/target_load)={}/{} shrink(source_warmup/target_load)={}/{}",
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_rebuild
                    .copy_db_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_rebuild
                    .target_bootstrap_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_rebuild
                    .target_load_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_rebuild_budget_change
                    .source_warmup_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_rebuild_budget_change
                    .target_load_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_metadata_realign
                    .append_turn_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .summary_metadata_realign
                    .target_load_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .window_shrink_catch_up
                    .source_warmup_ms
                    .range,
                3
            ),
            format_optional_decimal(
                report
                    .cold_path_phase_stability
                    .window_shrink_catch_up
                    .target_load_ms
                    .range,
                3
            ),
        );
    }
    println!(
        "entries window_only={} summary_window_cover={} summary_rebuild={} summary_rebuild_budget_change={} summary_metadata_realign={} summary_steady_state={} window_shrink_catch_up={} summary_chars(rebuild/budget_change/metadata/steady/shrink)={}/{}/{}/{}/{} payload_chars(window/cover/rebuild/budget_change/metadata/steady/shrink)={}/{}/{}/{}/{}/{}/{} approx_rss_step_delta_kib_p95(window/cover/rebuild/budget_change/metadata/steady/shrink/window_only_append_pre/append_pre/window_only_append_cold/append_cold/append)={}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{} shrink_source_window={} telemetry_scope={}",
        report.window_only_entry_count,
        report.summary_window_cover_entry_count,
        report.summary_rebuild_entry_count,
        report.summary_rebuild_budget_change_entry_count,
        report.summary_metadata_realign_entry_count,
        report.summary_steady_state_entry_count,
        report.window_shrink_catch_up_entry_count,
        report.summary_rebuild_summary_chars,
        report.summary_rebuild_budget_change_summary_chars,
        report.summary_metadata_realign_summary_chars,
        report.summary_steady_state_summary_chars,
        report.window_shrink_catch_up_summary_chars,
        report.window_only_payload_chars,
        report.summary_window_cover_payload_chars,
        report.summary_rebuild_payload_chars,
        report.summary_rebuild_budget_change_payload_chars,
        report.summary_metadata_realign_payload_chars,
        report.summary_steady_state_payload_chars,
        report.window_shrink_catch_up_payload_chars,
        format_optional_decimal(report.window_only_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_window_cover_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_rebuild_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_rebuild_budget_change_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_metadata_realign_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_steady_state_rss_delta_kib.p95, 1),
        format_optional_decimal(report.window_shrink_catch_up_rss_delta_kib.p95, 1),
        format_optional_decimal(report.window_only_append_pre_overflow_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_append_pre_overflow_rss_delta_kib.p95, 1),
        format_optional_decimal(report.window_only_append_cold_overflow_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_append_cold_overflow_rss_delta_kib.p95, 1),
        format_optional_decimal(report.summary_append_saturated_rss_delta_kib.p95, 1),
        report.window_shrink_source_window,
        report.rss_telemetry_scope
    );
    for warning in &report.gate.warnings {
        println!("warning: {warning}");
    }

    if enforce_gate && !report.gate.passed {
        return Err(format!(
            "memory context benchmark regression gate failed: {}",
            report.gate.reason.as_deref().unwrap_or("gate failed")
        ));
    }

    Ok(())
}

fn try_build_memory_context_benchmark_report(
    output_path: &str,
    benchmark_temp_root: &ResolvedMemoryContextBenchmarkTempRoot,
    history_turns: usize,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
    suite_repetitions: usize,
    enforce_gate: bool,
    normalized_min_speedup_ratio: f64,
) -> CliResult<MemoryContextBenchmarkReport> {
    let representative = suite_runs
        .last()
        .ok_or_else(|| "memory context benchmark requires at least one suite run".to_owned())?;
    let suite_p95_summaries = suite_runs
        .iter()
        .map(summarize_memory_context_suite_p95)
        .collect::<Vec<_>>();

    macro_rules! flatten_metric {
        ($field:ident) => {
            suite_runs
                .iter()
                .flat_map(|run| run.$field.iter().copied())
                .collect::<Vec<_>>()
        };
    }

    let window_only_samples = flatten_metric!(window_only_samples);
    let summary_window_cover_samples = flatten_metric!(summary_window_cover_samples);
    let summary_rebuild_samples = flatten_metric!(summary_rebuild_samples);
    let summary_rebuild_budget_change_samples =
        flatten_metric!(summary_rebuild_budget_change_samples);
    let summary_metadata_realign_samples = flatten_metric!(summary_metadata_realign_samples);
    let summary_steady_state_samples = flatten_metric!(summary_steady_state_samples);
    let window_shrink_catch_up_samples = flatten_metric!(window_shrink_catch_up_samples);
    let window_only_append_pre_overflow_samples =
        flatten_metric!(window_only_append_pre_overflow_samples);
    let window_only_append_cold_overflow_samples =
        flatten_metric!(window_only_append_cold_overflow_samples);
    let summary_append_pre_overflow_samples = flatten_metric!(summary_append_pre_overflow_samples);
    let summary_append_cold_overflow_samples =
        flatten_metric!(summary_append_cold_overflow_samples);
    let summary_append_saturated_samples = flatten_metric!(summary_append_saturated_samples);

    let window_only_rss_deltas_kib = flatten_metric!(window_only_rss_deltas_kib);
    let summary_window_cover_rss_deltas_kib = flatten_metric!(summary_window_cover_rss_deltas_kib);
    let summary_rebuild_rss_deltas_kib = flatten_metric!(summary_rebuild_rss_deltas_kib);
    let summary_rebuild_budget_change_rss_deltas_kib =
        flatten_metric!(summary_rebuild_budget_change_rss_deltas_kib);
    let summary_metadata_realign_rss_deltas_kib =
        flatten_metric!(summary_metadata_realign_rss_deltas_kib);
    let summary_steady_state_rss_deltas_kib = flatten_metric!(summary_steady_state_rss_deltas_kib);
    let window_shrink_catch_up_rss_deltas_kib =
        flatten_metric!(window_shrink_catch_up_rss_deltas_kib);
    let window_only_append_pre_overflow_rss_deltas_kib =
        flatten_metric!(window_only_append_pre_overflow_rss_deltas_kib);
    let window_only_append_cold_overflow_rss_deltas_kib =
        flatten_metric!(window_only_append_cold_overflow_rss_deltas_kib);
    let summary_append_pre_overflow_rss_deltas_kib =
        flatten_metric!(summary_append_pre_overflow_rss_deltas_kib);
    let summary_append_cold_overflow_rss_deltas_kib =
        flatten_metric!(summary_append_cold_overflow_rss_deltas_kib);
    let summary_append_saturated_rss_deltas_kib =
        flatten_metric!(summary_append_saturated_rss_deltas_kib);

    let window_only_latency_ms = compute_numeric_stats(&window_only_samples);
    let summary_window_cover_latency_ms = compute_numeric_stats(&summary_window_cover_samples);
    let summary_rebuild_latency_ms = compute_numeric_stats(&summary_rebuild_samples);
    let summary_rebuild_budget_change_latency_ms =
        compute_numeric_stats(&summary_rebuild_budget_change_samples);
    let summary_metadata_realign_latency_ms =
        compute_numeric_stats(&summary_metadata_realign_samples);
    let summary_steady_state_latency_ms = compute_numeric_stats(&summary_steady_state_samples);
    let window_shrink_catch_up_latency_ms = compute_numeric_stats(&window_shrink_catch_up_samples);
    let window_only_append_pre_overflow_latency_ms =
        compute_numeric_stats(&window_only_append_pre_overflow_samples);
    let window_only_append_cold_overflow_latency_ms =
        compute_numeric_stats(&window_only_append_cold_overflow_samples);
    let summary_append_pre_overflow_latency_ms =
        compute_numeric_stats(&summary_append_pre_overflow_samples);
    let summary_append_cold_overflow_latency_ms =
        compute_numeric_stats(&summary_append_cold_overflow_samples);
    let summary_append_saturated_latency_ms =
        compute_numeric_stats(&summary_append_saturated_samples);

    let window_only_rss_delta_kib = compute_numeric_stats(&window_only_rss_deltas_kib);
    let summary_window_cover_rss_delta_kib =
        compute_numeric_stats(&summary_window_cover_rss_deltas_kib);
    let summary_rebuild_rss_delta_kib = compute_numeric_stats(&summary_rebuild_rss_deltas_kib);
    let summary_rebuild_budget_change_rss_delta_kib =
        compute_numeric_stats(&summary_rebuild_budget_change_rss_deltas_kib);
    let summary_metadata_realign_rss_delta_kib =
        compute_numeric_stats(&summary_metadata_realign_rss_deltas_kib);
    let summary_steady_state_rss_delta_kib =
        compute_numeric_stats(&summary_steady_state_rss_deltas_kib);
    let window_shrink_catch_up_rss_delta_kib =
        compute_numeric_stats(&window_shrink_catch_up_rss_deltas_kib);
    let window_only_append_pre_overflow_rss_delta_kib =
        compute_numeric_stats(&window_only_append_pre_overflow_rss_deltas_kib);
    let window_only_append_cold_overflow_rss_delta_kib =
        compute_numeric_stats(&window_only_append_cold_overflow_rss_deltas_kib);
    let summary_append_pre_overflow_rss_delta_kib =
        compute_numeric_stats(&summary_append_pre_overflow_rss_deltas_kib);
    let summary_append_cold_overflow_rss_delta_kib =
        compute_numeric_stats(&summary_append_cold_overflow_rss_deltas_kib);
    let summary_append_saturated_rss_delta_kib =
        compute_numeric_stats(&summary_append_saturated_rss_deltas_kib);

    let summary_window_cover_vs_window_only_ratio_p95 = match (
        summary_window_cover_latency_ms.p95,
        window_only_latency_ms.p95,
    ) {
        (Some(cover_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
            Some(cover_p95 / window_only_p95)
        }
        _ => None,
    };
    let summary_window_cover_overhead_p95_ms = match (
        summary_window_cover_latency_ms.p95,
        window_only_latency_ms.p95,
    ) {
        (Some(cover_p95), Some(window_only_p95)) => Some(cover_p95 - window_only_p95),
        _ => None,
    };
    let summary_rebuild_budget_change_vs_rebuild_ratio_p95 = match (
        summary_rebuild_budget_change_latency_ms.p95,
        summary_rebuild_latency_ms.p95,
    ) {
        (Some(budget_change_p95), Some(rebuild_p95)) if rebuild_p95 > 0.0 => {
            Some(budget_change_p95 / rebuild_p95)
        }
        _ => None,
    };
    let summary_rebuild_budget_change_summary_char_growth_ratio =
        compute_weighted_summary_char_growth_ratio(
            suite_runs,
            |run| run.summary_rebuild_shape.summary_chars,
            |run| run.summary_rebuild_budget_change_shape.summary_chars,
            |run| {
                run.summary_rebuild_samples
                    .len()
                    .min(run.summary_rebuild_budget_change_samples.len())
            },
        );
    let summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95 =
        compute_workload_adjusted_ratio(
            summary_rebuild_budget_change_vs_rebuild_ratio_p95,
            summary_rebuild_budget_change_summary_char_growth_ratio,
        );
    let summary_metadata_realign_vs_budget_change_ratio_p95 = match (
        summary_metadata_realign_latency_ms.p95,
        summary_rebuild_budget_change_latency_ms.p95,
    ) {
        (Some(metadata_realign_p95), Some(budget_change_p95)) if budget_change_p95 > 0.0 => {
            Some(metadata_realign_p95 / budget_change_p95)
        }
        _ => None,
    };
    let speedup_ratio_p95 = match (
        summary_rebuild_latency_ms.p95,
        summary_steady_state_latency_ms.p95,
    ) {
        (Some(rebuild_p95), Some(steady_p95)) if steady_p95 > 0.0 => Some(rebuild_p95 / steady_p95),
        _ => None,
    };
    let window_shrink_catch_up_vs_rebuild_speedup_ratio_p95 = match (
        summary_rebuild_latency_ms.p95,
        window_shrink_catch_up_latency_ms.p95,
    ) {
        (Some(rebuild_p95), Some(shrink_p95)) if shrink_p95 > 0.0 => Some(rebuild_p95 / shrink_p95),
        _ => None,
    };
    let summary_append_pre_overflow_vs_window_only_ratio_p95 = match (
        summary_append_pre_overflow_latency_ms.p95,
        window_only_append_pre_overflow_latency_ms.p95,
    ) {
        (Some(summary_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
            Some(summary_p95 / window_only_p95)
        }
        _ => None,
    };
    let summary_append_cold_overflow_vs_window_only_ratio_p95 = match (
        summary_append_cold_overflow_latency_ms.p95,
        window_only_append_cold_overflow_latency_ms.p95,
    ) {
        (Some(summary_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
            Some(summary_p95 / window_only_p95)
        }
        _ => None,
    };

    let aggregated_p95_median_ms = MemoryContextAggregatedP95MedianMs {
        window_only: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only),
        ),
        summary_window_cover: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover),
        ),
        summary_rebuild: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild),
        ),
        summary_rebuild_budget_change: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild_budget_change),
        ),
        summary_metadata_realign: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_metadata_realign),
        ),
        summary_steady_state: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_steady_state),
        ),
        window_shrink_catch_up: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_shrink_catch_up),
        ),
        window_only_append_pre_overflow: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only_append_pre_overflow),
        ),
        window_only_append_cold_overflow: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only_append_cold_overflow),
        ),
        summary_append_pre_overflow: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_pre_overflow),
        ),
        summary_append_cold_overflow: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_cold_overflow),
        ),
        summary_append_saturated: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_saturated),
        ),
    };
    let suite_stability = build_memory_context_suite_stability_summary(&suite_p95_summaries);
    let cold_path_phases = build_memory_context_cold_path_phase_report(suite_runs);
    let cold_path_phase_stability =
        build_memory_context_cold_path_phase_stability_report(suite_runs);
    let cold_path_noise_attribution =
        build_memory_context_cold_path_noise_attribution_report(&cold_path_phase_stability);
    let cold_path_bootstrap_noise_attribution =
        build_memory_context_cold_path_bootstrap_noise_attribution_report(suite_runs);
    let cold_path_load_noise_attribution =
        build_memory_context_cold_path_load_noise_attribution_report(suite_runs);
    let prompt_efficiency_signals = build_memory_context_prompt_efficiency_signals(representative);
    let flattened_sample_ratios = MemoryContextRatioP95Summary {
        summary_window_cover_vs_window_only_ratio_p95,
        summary_window_cover_overhead_p95_ms,
        summary_rebuild_budget_change_vs_rebuild_ratio_p95,
        summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95,
        summary_metadata_realign_vs_budget_change_ratio_p95,
        speedup_ratio_p95,
        window_shrink_catch_up_vs_rebuild_speedup_ratio_p95,
        summary_append_pre_overflow_vs_window_only_ratio_p95,
        summary_append_cold_overflow_vs_window_only_ratio_p95,
    };
    let aggregated_ratios = MemoryContextRatioP95Summary {
        summary_window_cover_vs_window_only_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover_vs_window_only_ratio_p95),
        ),
        summary_window_cover_overhead_p95_ms: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover_overhead_p95_ms),
        ),
        summary_rebuild_budget_change_vs_rebuild_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild_budget_change_vs_rebuild_ratio_p95),
        ),
        summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95: median_option_f64(
            suite_p95_summaries.iter().map(|summary| {
                summary.summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95
            }),
        ),
        summary_metadata_realign_vs_budget_change_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_metadata_realign_vs_budget_change_ratio_p95),
        ),
        speedup_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.speedup_ratio_p95),
        ),
        window_shrink_catch_up_vs_rebuild_speedup_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_shrink_catch_up_vs_rebuild_speedup_ratio_p95),
        ),
        summary_append_pre_overflow_vs_window_only_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_pre_overflow_vs_window_only_ratio_p95),
        ),
        summary_append_cold_overflow_vs_window_only_ratio_p95: median_option_f64(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_cold_overflow_vs_window_only_ratio_p95),
        ),
    };
    let soft_warnings = build_memory_context_soft_warnings(
        aggregated_ratios.summary_window_cover_vs_window_only_ratio_p95,
        aggregated_ratios.summary_window_cover_overhead_p95_ms,
        summary_window_cover_samples.len(),
        suite_stability
            .window_only_p95_ms
            .range_over_p50
            .is_some_and(|range_over_p50| {
                range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            })
            || suite_stability
                .summary_window_cover_p95_ms
                .range_over_p50
                .is_some_and(|range_over_p50| {
                    range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
                })
            || suite_stability
                .summary_window_cover_vs_window_only_ratio_p95
                .range_over_p50
                .is_some_and(|range_over_p50| {
                    range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
                })
            || suite_stability
                .summary_window_cover_overhead_p95_ms
                .range_over_p50
                .is_some_and(|range_over_p50| {
                    range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
                }),
        aggregated_ratios.summary_rebuild_budget_change_vs_rebuild_ratio_p95,
        aggregated_ratios.summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95,
        summary_rebuild_samples
            .len()
            .min(summary_rebuild_budget_change_samples.len()),
        aggregated_ratios.summary_metadata_realign_vs_budget_change_ratio_p95,
        summary_metadata_realign_samples
            .len()
            .min(summary_rebuild_budget_change_samples.len()),
        suite_stability.speedup_ratio_p95.min,
        suite_stability.speedup_ratio_p95.range_over_p50,
        suite_stability.summary_rebuild_p95_ms.range_over_p50,
        suite_stability.summary_steady_state_p95_ms.p50,
        suite_stability.summary_steady_state_p95_ms.range,
        suite_stability.summary_steady_state_p95_ms.range_over_p50,
        suite_stability.speedup_ratio_p95.count,
        normalized_min_speedup_ratio,
        cold_path_noise_attribution.summary_rebuild.as_ref(),
        cold_path_bootstrap_noise_attribution
            .summary_rebuild
            .target_bootstrap
            .as_ref(),
        cold_path_load_noise_attribution
            .summary_rebuild
            .target_load
            .as_ref(),
        suite_stability
            .summary_rebuild_budget_change_p95_ms
            .range_over_p50,
        suite_stability
            .summary_metadata_realign_p95_ms
            .range_over_p50,
        suite_stability
            .summary_metadata_realign_vs_budget_change_ratio_p95
            .range_over_p50,
        benchmark_temp_root.source,
        &benchmark_temp_root.path,
    );

    let observed_speedup_ratio = aggregated_ratios.speedup_ratio_p95;
    let mut gate_reason = None;
    let gate_passed = if enforce_gate {
        match observed_speedup_ratio {
            Some(observed) if observed >= normalized_min_speedup_ratio => true,
            Some(observed) => {
                gate_reason = Some(format!(
                    "observed aggregated p95 speedup ratio {:.3} is below threshold {:.3}",
                    observed, normalized_min_speedup_ratio
                ));
                false
            }
            None => {
                gate_reason =
                    Some("unable to compute aggregated memory context speedup ratio".to_owned());
                false
            }
        }
    } else {
        true
    };

    Ok(MemoryContextBenchmarkReport {
        generated_at_epoch_s: current_epoch_seconds(),
        profile: "memory_context".to_owned(),
        output_path: output_path.to_owned(),
        benchmark_temp_root: benchmark_temp_root.path.display().to_string(),
        benchmark_temp_root_source: benchmark_temp_root.source,
        suite_repetitions,
        suite_aggregation: MEMORY_CONTEXT_SUITE_AGGREGATION_MEDIAN_OF_P95.to_owned(),
        rss_telemetry_scope: "best_effort_approx_process_rss_step_delta_via_ps".to_owned(),
        history_turns,
        sliding_window,
        window_shrink_source_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        seed_db_bytes: representative.seed_db_bytes,
        suite_p95_summaries,
        suite_stability,
        cold_path_phases,
        cold_path_phase_stability,
        cold_path_noise_attribution,
        cold_path_bootstrap_noise_attribution,
        cold_path_load_noise_attribution,
        window_only_latency_ms,
        summary_window_cover_latency_ms,
        summary_rebuild_latency_ms,
        summary_rebuild_budget_change_latency_ms,
        summary_metadata_realign_latency_ms,
        summary_steady_state_latency_ms,
        window_shrink_catch_up_latency_ms,
        window_only_append_pre_overflow_latency_ms,
        window_only_append_cold_overflow_latency_ms,
        summary_append_pre_overflow_latency_ms,
        summary_append_cold_overflow_latency_ms,
        summary_append_saturated_latency_ms,
        window_only_rss_delta_kib,
        summary_window_cover_rss_delta_kib,
        summary_rebuild_rss_delta_kib,
        summary_rebuild_budget_change_rss_delta_kib,
        summary_metadata_realign_rss_delta_kib,
        summary_steady_state_rss_delta_kib,
        window_shrink_catch_up_rss_delta_kib,
        window_only_append_pre_overflow_rss_delta_kib,
        window_only_append_cold_overflow_rss_delta_kib,
        summary_append_pre_overflow_rss_delta_kib,
        summary_append_cold_overflow_rss_delta_kib,
        summary_append_saturated_rss_delta_kib,
        window_only_entry_count: representative.window_only_shape.entry_count,
        window_only_turn_entries: representative.window_only_shape.turn_entries,
        window_only_payload_chars: representative.window_only_shape.payload_chars,
        summary_window_cover_entry_count: representative.summary_window_cover_shape.entry_count,
        summary_window_cover_turn_entries: representative.summary_window_cover_shape.turn_entries,
        summary_window_cover_payload_chars: representative.summary_window_cover_shape.payload_chars,
        summary_rebuild_entry_count: representative.summary_rebuild_shape.entry_count,
        summary_rebuild_turn_entries: representative.summary_rebuild_shape.turn_entries,
        summary_rebuild_summary_chars: representative.summary_rebuild_shape.summary_chars,
        summary_rebuild_payload_chars: representative.summary_rebuild_shape.payload_chars,
        summary_rebuild_budget_change_entry_count: representative
            .summary_rebuild_budget_change_shape
            .entry_count,
        summary_rebuild_budget_change_turn_entries: representative
            .summary_rebuild_budget_change_shape
            .turn_entries,
        summary_rebuild_budget_change_summary_chars: representative
            .summary_rebuild_budget_change_shape
            .summary_chars,
        summary_rebuild_budget_change_payload_chars: representative
            .summary_rebuild_budget_change_shape
            .payload_chars,
        summary_metadata_realign_entry_count: representative
            .summary_metadata_realign_shape
            .entry_count,
        summary_metadata_realign_turn_entries: representative
            .summary_metadata_realign_shape
            .turn_entries,
        summary_metadata_realign_summary_chars: representative
            .summary_metadata_realign_shape
            .summary_chars,
        summary_metadata_realign_payload_chars: representative
            .summary_metadata_realign_shape
            .payload_chars,
        summary_steady_state_entry_count: representative.summary_steady_state_shape.entry_count,
        summary_steady_state_turn_entries: representative.summary_steady_state_shape.turn_entries,
        summary_steady_state_summary_chars: representative.summary_steady_state_shape.summary_chars,
        summary_steady_state_payload_chars: representative.summary_steady_state_shape.payload_chars,
        window_shrink_catch_up_entry_count: representative.window_shrink_catch_up_shape.entry_count,
        window_shrink_catch_up_turn_entries: representative
            .window_shrink_catch_up_shape
            .turn_entries,
        window_shrink_catch_up_summary_chars: representative
            .window_shrink_catch_up_shape
            .summary_chars,
        window_shrink_catch_up_payload_chars: representative
            .window_shrink_catch_up_shape
            .payload_chars,
        prompt_efficiency_signals,
        flattened_sample_ratios,
        aggregated_p95_median_ms,
        aggregated_ratios,
        gate: MemoryContextBenchmarkGateSummary {
            enforced: enforce_gate,
            passed: gate_passed,
            min_steady_state_speedup_ratio: normalized_min_speedup_ratio,
            observed_speedup_ratio,
            summary_window_cover_soft_max_ratio_p95:
                DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_RATIO_P95,
            summary_window_cover_soft_max_overhead_p95_ms:
                DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_OVERHEAD_P95_MS,
            summary_window_cover_soft_warning_min_samples:
                DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_WARNING_MIN_SAMPLES,
            summary_rebuild_budget_change_vs_rebuild_soft_max_ratio_p95:
                DEFAULT_MEMORY_CONTEXT_REBUILD_BUDGET_CHANGE_SOFT_MAX_RATIO_P95,
            summary_metadata_realign_vs_budget_change_soft_max_ratio_p95:
                DEFAULT_MEMORY_CONTEXT_METADATA_REALIGN_SOFT_MAX_RATIO_P95,
            suite_stability_soft_warning_min_suites:
                DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_WARNING_MIN_SUITES,
            suite_stability_soft_max_range_over_p50:
                DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50,
            warnings: soft_warnings,
            reason: gate_reason,
        },
    })
}

#[cfg(test)]
fn build_memory_context_benchmark_report(
    output_path: &str,
    benchmark_temp_root: &ResolvedMemoryContextBenchmarkTempRoot,
    history_turns: usize,
    sliding_window: usize,
    window_shrink_source_window: usize,
    summary_max_chars: usize,
    words_per_turn: usize,
    rebuild_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
    suite_repetitions: usize,
    enforce_gate: bool,
    normalized_min_speedup_ratio: f64,
) -> MemoryContextBenchmarkReport {
    try_build_memory_context_benchmark_report(
        output_path,
        benchmark_temp_root,
        history_turns,
        sliding_window,
        window_shrink_source_window,
        summary_max_chars,
        words_per_turn,
        rebuild_iterations,
        hot_iterations,
        warmup_iterations,
        suite_runs,
        suite_repetitions,
        enforce_gate,
        normalized_min_speedup_ratio,
    )
    .expect("memory context benchmark report should build")
}

fn summarize_memory_context_suite_p95(
    run: &MemoryContextBenchmarkSuiteSamples,
) -> MemoryContextSuiteP95Summary {
    let window_only = compute_numeric_stats(&run.window_only_samples).p95;
    let summary_window_cover = compute_numeric_stats(&run.summary_window_cover_samples).p95;
    let summary_rebuild = compute_numeric_stats(&run.summary_rebuild_samples).p95;
    let summary_rebuild_budget_change =
        compute_numeric_stats(&run.summary_rebuild_budget_change_samples).p95;
    let summary_metadata_realign = compute_numeric_stats(&run.summary_metadata_realign_samples).p95;
    let summary_steady_state = compute_numeric_stats(&run.summary_steady_state_samples).p95;
    let window_shrink_catch_up = compute_numeric_stats(&run.window_shrink_catch_up_samples).p95;
    let window_only_append_pre_overflow =
        compute_numeric_stats(&run.window_only_append_pre_overflow_samples).p95;
    let window_only_append_cold_overflow =
        compute_numeric_stats(&run.window_only_append_cold_overflow_samples).p95;
    let summary_append_pre_overflow =
        compute_numeric_stats(&run.summary_append_pre_overflow_samples).p95;
    let summary_append_cold_overflow =
        compute_numeric_stats(&run.summary_append_cold_overflow_samples).p95;
    let summary_append_saturated = compute_numeric_stats(&run.summary_append_saturated_samples).p95;

    let summary_window_cover_vs_window_only_ratio_p95 = match (summary_window_cover, window_only) {
        (Some(cover_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
            Some(cover_p95 / window_only_p95)
        }
        _ => None,
    };
    let summary_window_cover_overhead_p95_ms = match (summary_window_cover, window_only) {
        (Some(cover_p95), Some(window_only_p95)) => Some(cover_p95 - window_only_p95),
        _ => None,
    };
    let summary_rebuild_budget_change_vs_rebuild_ratio_p95 =
        match (summary_rebuild_budget_change, summary_rebuild) {
            (Some(budget_change_p95), Some(rebuild_p95)) if rebuild_p95 > 0.0 => {
                Some(budget_change_p95 / rebuild_p95)
            }
            _ => None,
        };
    let summary_rebuild_budget_change_summary_char_growth_ratio = compute_summary_char_growth_ratio(
        run.summary_rebuild_shape.summary_chars,
        run.summary_rebuild_budget_change_shape.summary_chars,
    );
    let summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95 =
        compute_workload_adjusted_ratio(
            summary_rebuild_budget_change_vs_rebuild_ratio_p95,
            summary_rebuild_budget_change_summary_char_growth_ratio,
        );
    let summary_metadata_realign_vs_budget_change_ratio_p95 =
        match (summary_metadata_realign, summary_rebuild_budget_change) {
            (Some(metadata_realign_p95), Some(budget_change_p95)) if budget_change_p95 > 0.0 => {
                Some(metadata_realign_p95 / budget_change_p95)
            }
            _ => None,
        };
    let speedup_ratio_p95 = match (summary_rebuild, summary_steady_state) {
        (Some(rebuild_p95), Some(steady_p95)) if steady_p95 > 0.0 => Some(rebuild_p95 / steady_p95),
        _ => None,
    };
    let window_shrink_catch_up_vs_rebuild_speedup_ratio_p95 =
        match (summary_rebuild, window_shrink_catch_up) {
            (Some(rebuild_p95), Some(shrink_p95)) if shrink_p95 > 0.0 => {
                Some(rebuild_p95 / shrink_p95)
            }
            _ => None,
        };
    let summary_append_pre_overflow_vs_window_only_ratio_p95 =
        match (summary_append_pre_overflow, window_only_append_pre_overflow) {
            (Some(summary_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
                Some(summary_p95 / window_only_p95)
            }
            _ => None,
        };
    let summary_append_cold_overflow_vs_window_only_ratio_p95 = match (
        summary_append_cold_overflow,
        window_only_append_cold_overflow,
    ) {
        (Some(summary_p95), Some(window_only_p95)) if window_only_p95 > 0.0 => {
            Some(summary_p95 / window_only_p95)
        }
        _ => None,
    };

    MemoryContextSuiteP95Summary {
        window_only,
        summary_window_cover,
        summary_rebuild,
        summary_rebuild_budget_change,
        summary_metadata_realign,
        summary_steady_state,
        window_shrink_catch_up,
        window_only_append_pre_overflow,
        window_only_append_cold_overflow,
        summary_append_pre_overflow,
        summary_append_cold_overflow,
        summary_append_saturated,
        summary_window_cover_vs_window_only_ratio_p95,
        summary_window_cover_overhead_p95_ms,
        summary_rebuild_budget_change_vs_rebuild_ratio_p95,
        summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95,
        summary_metadata_realign_vs_budget_change_ratio_p95,
        speedup_ratio_p95,
        window_shrink_catch_up_vs_rebuild_speedup_ratio_p95,
        summary_append_pre_overflow_vs_window_only_ratio_p95,
        summary_append_cold_overflow_vs_window_only_ratio_p95,
    }
}

fn build_memory_context_suite_stability_summary(
    suite_p95_summaries: &[MemoryContextSuiteP95Summary],
) -> MemoryContextSuiteStabilitySummary {
    MemoryContextSuiteStabilitySummary {
        window_only_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only),
        ),
        summary_window_cover_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover),
        ),
        summary_rebuild_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild),
        ),
        summary_rebuild_budget_change_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild_budget_change),
        ),
        summary_metadata_realign_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_metadata_realign),
        ),
        summary_steady_state_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_steady_state),
        ),
        window_shrink_catch_up_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_shrink_catch_up),
        ),
        window_only_append_pre_overflow_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only_append_pre_overflow),
        ),
        window_only_append_cold_overflow_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_only_append_cold_overflow),
        ),
        summary_append_pre_overflow_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_pre_overflow),
        ),
        summary_append_cold_overflow_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_cold_overflow),
        ),
        summary_append_saturated_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_saturated),
        ),
        summary_window_cover_vs_window_only_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover_vs_window_only_ratio_p95),
        ),
        summary_window_cover_overhead_p95_ms: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_window_cover_overhead_p95_ms),
        ),
        summary_rebuild_budget_change_vs_rebuild_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_rebuild_budget_change_vs_rebuild_ratio_p95),
        ),
        summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95:
            compute_option_numeric_spread(suite_p95_summaries.iter().map(|summary| {
                summary.summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95
            })),
        summary_metadata_realign_vs_budget_change_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_metadata_realign_vs_budget_change_ratio_p95),
        ),
        speedup_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.speedup_ratio_p95),
        ),
        window_shrink_catch_up_vs_rebuild_speedup_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.window_shrink_catch_up_vs_rebuild_speedup_ratio_p95),
        ),
        summary_append_pre_overflow_vs_window_only_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_pre_overflow_vs_window_only_ratio_p95),
        ),
        summary_append_cold_overflow_vs_window_only_ratio_p95: compute_option_numeric_spread(
            suite_p95_summaries
                .iter()
                .map(|summary| summary.summary_append_cold_overflow_vs_window_only_ratio_p95),
        ),
    }
}

fn build_memory_context_cold_path_phase_report(
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
) -> MemoryContextColdPathPhaseReport {
    MemoryContextColdPathPhaseReport {
        summary_rebuild: build_memory_context_cold_path_phase_stats(
            suite_runs
                .iter()
                .map(|run| &run.summary_rebuild_phase_samples),
        ),
        summary_rebuild_budget_change: build_memory_context_cold_path_phase_stats(
            suite_runs
                .iter()
                .map(|run| &run.summary_rebuild_budget_change_phase_samples),
        ),
        summary_metadata_realign: build_memory_context_cold_path_phase_stats(
            suite_runs
                .iter()
                .map(|run| &run.summary_metadata_realign_phase_samples),
        ),
        window_shrink_catch_up: build_memory_context_cold_path_phase_stats(
            suite_runs
                .iter()
                .map(|run| &run.window_shrink_catch_up_phase_samples),
        ),
    }
}

fn build_memory_context_cold_path_phase_stability_report(
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
) -> MemoryContextColdPathPhaseStabilityReport {
    MemoryContextColdPathPhaseStabilityReport {
        summary_rebuild: build_memory_context_cold_path_phase_stability_summary(
            suite_runs
                .iter()
                .map(|run| &run.summary_rebuild_phase_samples),
        ),
        summary_rebuild_budget_change: build_memory_context_cold_path_phase_stability_summary(
            suite_runs
                .iter()
                .map(|run| &run.summary_rebuild_budget_change_phase_samples),
        ),
        summary_metadata_realign: build_memory_context_cold_path_phase_stability_summary(
            suite_runs
                .iter()
                .map(|run| &run.summary_metadata_realign_phase_samples),
        ),
        window_shrink_catch_up: build_memory_context_cold_path_phase_stability_summary(
            suite_runs
                .iter()
                .map(|run| &run.window_shrink_catch_up_phase_samples),
        ),
    }
}

fn build_memory_context_cold_path_noise_attribution_report(
    stability: &MemoryContextColdPathPhaseStabilityReport,
) -> MemoryContextColdPathNoiseAttributionReport {
    MemoryContextColdPathNoiseAttributionReport {
        summary_rebuild: dominant_memory_context_cold_path_noise(&stability.summary_rebuild),
        summary_rebuild_budget_change: dominant_memory_context_cold_path_noise(
            &stability.summary_rebuild_budget_change,
        ),
        summary_metadata_realign: dominant_memory_context_cold_path_noise(
            &stability.summary_metadata_realign,
        ),
        window_shrink_catch_up: dominant_memory_context_cold_path_noise(
            &stability.window_shrink_catch_up,
        ),
    }
}

fn dominant_memory_context_cold_path_noise(
    stability: &MemoryContextColdPathPhaseStabilitySummary,
) -> Option<MemoryContextColdPathNoiseAttribution> {
    [
        ("copy_db_ms", stability.copy_db_ms.range_over_p50),
        (
            "source_bootstrap_ms",
            stability.source_bootstrap_ms.range_over_p50,
        ),
        (
            "source_warmup_ms",
            stability.source_warmup_ms.range_over_p50,
        ),
        ("append_turn_ms", stability.append_turn_ms.range_over_p50),
        (
            "target_bootstrap_ms",
            stability.target_bootstrap_ms.range_over_p50,
        ),
        ("target_load_ms", stability.target_load_ms.range_over_p50),
    ]
    .into_iter()
    .filter_map(|(phase, range_over_p50)| {
        range_over_p50.map(|range_over_p50| MemoryContextColdPathNoiseAttribution {
            phase: phase.to_owned(),
            range_over_p50,
        })
    })
    .max_by(|left, right| left.range_over_p50.total_cmp(&right.range_over_p50))
}

fn build_memory_context_cold_path_bootstrap_noise_attribution_report(
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
) -> MemoryContextColdPathBootstrapNoiseAttributionReport {
    MemoryContextColdPathBootstrapNoiseAttributionReport {
        summary_rebuild: MemoryContextColdPathBootstrapNoiseAttribution {
            source_bootstrap: None,
            target_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_rebuild_phase_samples),
                MemoryContextBootstrapKind::Target,
            ),
        },
        summary_rebuild_budget_change: MemoryContextColdPathBootstrapNoiseAttribution {
            source_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_rebuild_budget_change_phase_samples),
                MemoryContextBootstrapKind::Source,
            ),
            target_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_rebuild_budget_change_phase_samples),
                MemoryContextBootstrapKind::Target,
            ),
        },
        summary_metadata_realign: MemoryContextColdPathBootstrapNoiseAttribution {
            source_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_metadata_realign_phase_samples),
                MemoryContextBootstrapKind::Source,
            ),
            target_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_metadata_realign_phase_samples),
                MemoryContextBootstrapKind::Target,
            ),
        },
        window_shrink_catch_up: MemoryContextColdPathBootstrapNoiseAttribution {
            source_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.window_shrink_catch_up_phase_samples),
                MemoryContextBootstrapKind::Source,
            ),
            target_bootstrap: dominant_memory_context_bootstrap_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.window_shrink_catch_up_phase_samples),
                MemoryContextBootstrapKind::Target,
            ),
        },
    }
}

fn build_memory_context_cold_path_load_noise_attribution_report(
    suite_runs: &[MemoryContextBenchmarkSuiteSamples],
) -> MemoryContextColdPathLoadNoiseAttributionReport {
    MemoryContextColdPathLoadNoiseAttributionReport {
        summary_rebuild: MemoryContextColdPathLoadNoiseAttribution {
            target_load: dominant_memory_context_load_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_rebuild_phase_samples),
            ),
        },
        summary_rebuild_budget_change: MemoryContextColdPathLoadNoiseAttribution {
            target_load: dominant_memory_context_load_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_rebuild_budget_change_phase_samples),
            ),
        },
        summary_metadata_realign: MemoryContextColdPathLoadNoiseAttribution {
            target_load: dominant_memory_context_load_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.summary_metadata_realign_phase_samples),
            ),
        },
        window_shrink_catch_up: MemoryContextColdPathLoadNoiseAttribution {
            target_load: dominant_memory_context_load_noise(
                suite_runs
                    .iter()
                    .map(|run| &run.window_shrink_catch_up_phase_samples),
            ),
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum MemoryContextBootstrapKind {
    Source,
    Target,
}

#[derive(Debug, Clone, Default)]
struct MemoryContextBootstrapSubphaseSuiteP95Summary {
    normalize_path_ms: Option<f64>,
    registry_lock_ms: Option<f64>,
    registry_lookup_ms: Option<f64>,
    runtime_create_ms: Option<f64>,
    parent_dir_create_ms: Option<f64>,
    connection_open_ms: Option<f64>,
    configure_connection_ms: Option<f64>,
    schema_init_ms: Option<f64>,
    schema_upgrade_ms: Option<f64>,
    registry_insert_ms: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct MemoryContextLoadSubphaseSuiteP95Summary {
    window_query_ms: Option<f64>,
    window_turn_count_query_ms: Option<f64>,
    window_exact_rows_query_ms: Option<f64>,
    window_known_overflow_rows_query_ms: Option<f64>,
    window_fallback_rows_query_ms: Option<f64>,
    summary_checkpoint_meta_query_ms: Option<f64>,
    summary_checkpoint_body_load_ms: Option<f64>,
    summary_checkpoint_metadata_update_ms: Option<f64>,
    summary_checkpoint_metadata_update_returning_body_ms: Option<f64>,
    summary_rebuild_ms: Option<f64>,
    summary_rebuild_stream_ms: Option<f64>,
    summary_rebuild_checkpoint_upsert_ms: Option<f64>,
    summary_rebuild_checkpoint_metadata_upsert_ms: Option<f64>,
    summary_rebuild_checkpoint_body_upsert_ms: Option<f64>,
    summary_rebuild_checkpoint_commit_ms: Option<f64>,
    summary_catch_up_ms: Option<f64>,
}

fn dominant_memory_context_bootstrap_noise<'a>(
    phase_samples: impl Iterator<Item = &'a MemoryContextColdPathPhaseSamples>,
    bootstrap_kind: MemoryContextBootstrapKind,
) -> Option<MemoryContextBootstrapNoiseAttribution> {
    let suite_p95 = phase_samples
        .map(|samples| memory_context_bootstrap_subphase_suite_p95(samples, bootstrap_kind))
        .collect::<Vec<_>>();

    [
        (
            "normalize_path_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.normalize_path_ms),
            )
            .range_over_p50,
        ),
        (
            "registry_lock_ms",
            compute_option_numeric_spread(suite_p95.iter().map(|summary| summary.registry_lock_ms))
                .range_over_p50,
        ),
        (
            "registry_lookup_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.registry_lookup_ms),
            )
            .range_over_p50,
        ),
        (
            "runtime_create_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.runtime_create_ms),
            )
            .range_over_p50,
        ),
        (
            "parent_dir_create_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.parent_dir_create_ms),
            )
            .range_over_p50,
        ),
        (
            "connection_open_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.connection_open_ms),
            )
            .range_over_p50,
        ),
        (
            "configure_connection_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.configure_connection_ms),
            )
            .range_over_p50,
        ),
        (
            "schema_init_ms",
            compute_option_numeric_spread(suite_p95.iter().map(|summary| summary.schema_init_ms))
                .range_over_p50,
        ),
        (
            "schema_upgrade_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.schema_upgrade_ms),
            )
            .range_over_p50,
        ),
        (
            "registry_insert_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.registry_insert_ms),
            )
            .range_over_p50,
        ),
    ]
    .into_iter()
    .filter_map(|(phase, range_over_p50)| {
        range_over_p50.map(|range_over_p50| MemoryContextBootstrapNoiseAttribution {
            phase: phase.to_owned(),
            range_over_p50,
        })
    })
    .max_by(|left, right| left.range_over_p50.total_cmp(&right.range_over_p50))
}

fn dominant_memory_context_load_noise<'a>(
    phase_samples: impl Iterator<Item = &'a MemoryContextColdPathPhaseSamples>,
) -> Option<MemoryContextLoadNoiseAttribution> {
    let suite_p95 = phase_samples
        .map(memory_context_load_subphase_suite_p95)
        .collect::<Vec<_>>();

    [
        (
            "window_query_ms",
            compute_option_numeric_spread(suite_p95.iter().map(|summary| summary.window_query_ms))
                .range_over_p50,
        ),
        (
            "window_turn_count_query_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.window_turn_count_query_ms),
            )
            .range_over_p50,
        ),
        (
            "window_exact_rows_query_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.window_exact_rows_query_ms),
            )
            .range_over_p50,
        ),
        (
            "window_known_overflow_rows_query_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.window_known_overflow_rows_query_ms),
            )
            .range_over_p50,
        ),
        (
            "window_fallback_rows_query_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.window_fallback_rows_query_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_checkpoint_meta_query_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_checkpoint_meta_query_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_checkpoint_body_load_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_checkpoint_body_load_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_checkpoint_metadata_update_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_checkpoint_metadata_update_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_checkpoint_metadata_update_returning_body_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_checkpoint_metadata_update_returning_body_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.summary_rebuild_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_stream_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_rebuild_stream_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_checkpoint_upsert_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_rebuild_checkpoint_upsert_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_checkpoint_metadata_upsert_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_rebuild_checkpoint_metadata_upsert_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_checkpoint_body_upsert_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_rebuild_checkpoint_body_upsert_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_rebuild_checkpoint_commit_ms",
            compute_option_numeric_spread(
                suite_p95
                    .iter()
                    .map(|summary| summary.summary_rebuild_checkpoint_commit_ms),
            )
            .range_over_p50,
        ),
        (
            "summary_catch_up_ms",
            compute_option_numeric_spread(
                suite_p95.iter().map(|summary| summary.summary_catch_up_ms),
            )
            .range_over_p50,
        ),
    ]
    .into_iter()
    .filter_map(|(phase, range_over_p50)| {
        range_over_p50.map(|range_over_p50| MemoryContextLoadNoiseAttribution {
            phase: phase.to_owned(),
            range_over_p50,
        })
    })
    .max_by(|left, right| left.range_over_p50.total_cmp(&right.range_over_p50))
}

fn memory_context_bootstrap_subphase_suite_p95(
    samples: &MemoryContextColdPathPhaseSamples,
    bootstrap_kind: MemoryContextBootstrapKind,
) -> MemoryContextBootstrapSubphaseSuiteP95Summary {
    match bootstrap_kind {
        MemoryContextBootstrapKind::Source => MemoryContextBootstrapSubphaseSuiteP95Summary {
            normalize_path_ms: compute_numeric_stats(&samples.source_bootstrap_normalize_path_ms)
                .p95,
            registry_lock_ms: compute_numeric_stats(&samples.source_bootstrap_registry_lock_ms).p95,
            registry_lookup_ms: compute_numeric_stats(&samples.source_bootstrap_registry_lookup_ms)
                .p95,
            runtime_create_ms: compute_numeric_stats(&samples.source_bootstrap_runtime_create_ms)
                .p95,
            parent_dir_create_ms: compute_numeric_stats(
                &samples.source_bootstrap_parent_dir_create_ms,
            )
            .p95,
            connection_open_ms: compute_numeric_stats(&samples.source_bootstrap_connection_open_ms)
                .p95,
            configure_connection_ms: compute_numeric_stats(
                &samples.source_bootstrap_configure_connection_ms,
            )
            .p95,
            schema_init_ms: compute_numeric_stats(&samples.source_bootstrap_schema_init_ms).p95,
            schema_upgrade_ms: compute_numeric_stats(&samples.source_bootstrap_schema_upgrade_ms)
                .p95,
            registry_insert_ms: compute_numeric_stats(&samples.source_bootstrap_registry_insert_ms)
                .p95,
        },
        MemoryContextBootstrapKind::Target => MemoryContextBootstrapSubphaseSuiteP95Summary {
            normalize_path_ms: compute_numeric_stats(&samples.target_bootstrap_normalize_path_ms)
                .p95,
            registry_lock_ms: compute_numeric_stats(&samples.target_bootstrap_registry_lock_ms).p95,
            registry_lookup_ms: compute_numeric_stats(&samples.target_bootstrap_registry_lookup_ms)
                .p95,
            runtime_create_ms: compute_numeric_stats(&samples.target_bootstrap_runtime_create_ms)
                .p95,
            parent_dir_create_ms: compute_numeric_stats(
                &samples.target_bootstrap_parent_dir_create_ms,
            )
            .p95,
            connection_open_ms: compute_numeric_stats(&samples.target_bootstrap_connection_open_ms)
                .p95,
            configure_connection_ms: compute_numeric_stats(
                &samples.target_bootstrap_configure_connection_ms,
            )
            .p95,
            schema_init_ms: compute_numeric_stats(&samples.target_bootstrap_schema_init_ms).p95,
            schema_upgrade_ms: compute_numeric_stats(&samples.target_bootstrap_schema_upgrade_ms)
                .p95,
            registry_insert_ms: compute_numeric_stats(&samples.target_bootstrap_registry_insert_ms)
                .p95,
        },
    }
}

fn memory_context_load_subphase_suite_p95(
    samples: &MemoryContextColdPathPhaseSamples,
) -> MemoryContextLoadSubphaseSuiteP95Summary {
    MemoryContextLoadSubphaseSuiteP95Summary {
        window_query_ms: compute_numeric_stats(&samples.target_load_window_query_ms).p95,
        window_turn_count_query_ms: compute_numeric_stats(
            &samples.target_load_window_turn_count_query_ms,
        )
        .p95,
        window_exact_rows_query_ms: compute_numeric_stats(
            &samples.target_load_window_exact_rows_query_ms,
        )
        .p95,
        window_known_overflow_rows_query_ms: compute_numeric_stats(
            &samples.target_load_window_known_overflow_rows_query_ms,
        )
        .p95,
        window_fallback_rows_query_ms: compute_numeric_stats(
            &samples.target_load_window_fallback_rows_query_ms,
        )
        .p95,
        summary_checkpoint_meta_query_ms: compute_numeric_stats(
            &samples.target_load_summary_checkpoint_meta_query_ms,
        )
        .p95,
        summary_checkpoint_body_load_ms: compute_numeric_stats(
            &samples.target_load_summary_checkpoint_body_load_ms,
        )
        .p95,
        summary_checkpoint_metadata_update_ms: compute_numeric_stats(
            &samples.target_load_summary_checkpoint_metadata_update_ms,
        )
        .p95,
        summary_checkpoint_metadata_update_returning_body_ms: compute_numeric_stats(
            &samples.target_load_summary_checkpoint_metadata_update_returning_body_ms,
        )
        .p95,
        summary_rebuild_ms: compute_numeric_stats(&samples.target_load_summary_rebuild_ms).p95,
        summary_rebuild_stream_ms: compute_numeric_stats(
            &samples.target_load_summary_rebuild_stream_ms,
        )
        .p95,
        summary_rebuild_checkpoint_upsert_ms: compute_numeric_stats(
            &samples.target_load_summary_rebuild_checkpoint_upsert_ms,
        )
        .p95,
        summary_rebuild_checkpoint_metadata_upsert_ms: compute_numeric_stats(
            &samples.target_load_summary_rebuild_checkpoint_metadata_upsert_ms,
        )
        .p95,
        summary_rebuild_checkpoint_body_upsert_ms: compute_numeric_stats(
            &samples.target_load_summary_rebuild_checkpoint_body_upsert_ms,
        )
        .p95,
        summary_rebuild_checkpoint_commit_ms: compute_numeric_stats(
            &samples.target_load_summary_rebuild_checkpoint_commit_ms,
        )
        .p95,
        summary_catch_up_ms: compute_numeric_stats(&samples.target_load_summary_catch_up_ms).p95,
    }
}

fn build_memory_context_cold_path_phase_stats<'a>(
    phase_samples: impl Iterator<Item = &'a MemoryContextColdPathPhaseSamples>,
) -> MemoryContextColdPathPhaseStats {
    let merged = merge_memory_context_cold_path_phase_samples(phase_samples);
    MemoryContextColdPathPhaseStats {
        copy_db_ms: compute_numeric_stats(&merged.copy_db_ms),
        source_bootstrap_ms: compute_numeric_stats(&merged.source_bootstrap_ms),
        source_warmup_ms: compute_numeric_stats(&merged.source_warmup_ms),
        append_turn_ms: compute_numeric_stats(&merged.append_turn_ms),
        target_bootstrap_ms: compute_numeric_stats(&merged.target_bootstrap_ms),
        target_load_ms: compute_numeric_stats(&merged.target_load_ms),
    }
}

fn build_memory_context_cold_path_phase_stability_summary<'a>(
    phase_samples: impl Iterator<Item = &'a MemoryContextColdPathPhaseSamples>,
) -> MemoryContextColdPathPhaseStabilitySummary {
    let suite_p95 = phase_samples
        .map(memory_context_cold_path_phase_suite_p95)
        .collect::<Vec<_>>();
    MemoryContextColdPathPhaseStabilitySummary {
        copy_db_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.copy_db_ms),
        ),
        source_bootstrap_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.source_bootstrap_ms),
        ),
        source_warmup_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.source_warmup_ms),
        ),
        append_turn_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.append_turn_ms),
        ),
        target_bootstrap_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.target_bootstrap_ms),
        ),
        target_load_ms: compute_option_numeric_spread(
            suite_p95.iter().map(|summary| summary.target_load_ms),
        ),
    }
}

fn merge_memory_context_cold_path_phase_samples<'a>(
    phase_samples: impl Iterator<Item = &'a MemoryContextColdPathPhaseSamples>,
) -> MemoryContextColdPathPhaseSamples {
    let mut merged = MemoryContextColdPathPhaseSamples::default();
    for sample in phase_samples {
        merged.copy_db_ms.extend_from_slice(&sample.copy_db_ms);
        merged
            .source_bootstrap_ms
            .extend_from_slice(&sample.source_bootstrap_ms);
        merged
            .source_warmup_ms
            .extend_from_slice(&sample.source_warmup_ms);
        merged
            .append_turn_ms
            .extend_from_slice(&sample.append_turn_ms);
        merged
            .target_bootstrap_ms
            .extend_from_slice(&sample.target_bootstrap_ms);
        merged
            .target_load_ms
            .extend_from_slice(&sample.target_load_ms);
    }
    merged
}

fn memory_context_cold_path_phase_suite_p95(
    phase_samples: &MemoryContextColdPathPhaseSamples,
) -> MemoryContextColdPathPhaseSuiteP95Summary {
    MemoryContextColdPathPhaseSuiteP95Summary {
        copy_db_ms: compute_numeric_stats(&phase_samples.copy_db_ms).p95,
        source_bootstrap_ms: compute_numeric_stats(&phase_samples.source_bootstrap_ms).p95,
        source_warmup_ms: compute_numeric_stats(&phase_samples.source_warmup_ms).p95,
        append_turn_ms: compute_numeric_stats(&phase_samples.append_turn_ms).p95,
        target_bootstrap_ms: compute_numeric_stats(&phase_samples.target_bootstrap_ms).p95,
        target_load_ms: compute_numeric_stats(&phase_samples.target_load_ms).p95,
    }
}

#[derive(Debug, Clone, Default)]
struct MemoryContextColdPathPhaseSuiteP95Summary {
    copy_db_ms: Option<f64>,
    source_bootstrap_ms: Option<f64>,
    source_warmup_ms: Option<f64>,
    append_turn_ms: Option<f64>,
    target_bootstrap_ms: Option<f64>,
    target_load_ms: Option<f64>,
}

fn median_option_f64<I>(values: I) -> Option<f64>
where
    I: IntoIterator<Item = Option<f64>>,
{
    let present = values.into_iter().flatten().collect::<Vec<_>>();
    compute_numeric_stats(&present).p50
}

fn compute_option_numeric_spread<I>(values: I) -> NumericSpreadSummary
where
    I: IntoIterator<Item = Option<f64>>,
{
    let present = values.into_iter().flatten().collect::<Vec<_>>();
    compute_numeric_spread(&present)
}

fn compute_numeric_spread(values: &[f64]) -> NumericSpreadSummary {
    let stats = compute_numeric_stats(values);
    let range = match (stats.min, stats.max) {
        (Some(min), Some(max)) => Some(normalize_spread_delta(max - min)),
        _ => None,
    };
    let range_over_p50 = match (range, stats.p50) {
        (Some(range), Some(p50)) if p50 > 0.0 => Some(range / p50),
        _ => None,
    };
    let max_over_p50 = match (stats.max, stats.p50) {
        (Some(max), Some(p50)) if p50 > 0.0 => Some(max / p50),
        _ => None,
    };

    NumericSpreadSummary {
        count: stats.count,
        min: stats.min,
        p50: stats.p50,
        max: stats.max,
        range,
        range_over_p50,
        max_over_p50,
    }
}

fn normalize_spread_delta(value: f64) -> f64 {
    if value.abs() < 1e-12 { 0.0 } else { value }
}

fn compute_summary_char_growth_ratio(
    base_summary_chars: usize,
    target_summary_chars: usize,
) -> Option<f64> {
    match (base_summary_chars, target_summary_chars) {
        (base, target) if base > 0 && target > 0 => Some((target as f64 / base as f64).max(1.0)),
        _ => None,
    }
}

fn compute_ratio_f64(numerator: usize, denominator: usize) -> Option<f64> {
    if denominator == 0 {
        return None;
    }

    let numerator = numerator as f64;
    let denominator = denominator as f64;

    Some(numerator / denominator)
}

fn compute_workload_adjusted_ratio(
    raw_ratio_p95: Option<f64>,
    summary_char_growth_ratio: Option<f64>,
) -> Option<f64> {
    match (raw_ratio_p95, summary_char_growth_ratio) {
        (Some(raw_ratio_p95), Some(summary_char_growth_ratio))
            if summary_char_growth_ratio > 0.0 =>
        {
            Some(raw_ratio_p95 / summary_char_growth_ratio)
        }
        _ => None,
    }
}

fn compute_weighted_summary_char_growth_ratio<T, Base, Target, Weight>(
    values: &[T],
    base_summary_chars: Base,
    target_summary_chars: Target,
    comparable_sample_count: Weight,
) -> Option<f64>
where
    Base: Fn(&T) -> usize,
    Target: Fn(&T) -> usize,
    Weight: Fn(&T) -> usize,
{
    let (weighted_base_summary_chars, weighted_target_summary_chars) =
        values
            .iter()
            .fold((0usize, 0usize), |(base_acc, target_acc), value| {
                let comparable_sample_count = comparable_sample_count(value);
                let base_summary_chars = base_summary_chars(value);
                let target_summary_chars = target_summary_chars(value);
                if comparable_sample_count == 0
                    || base_summary_chars == 0
                    || target_summary_chars == 0
                {
                    return (base_acc, target_acc);
                }

                (
                    base_acc
                        .saturating_add(base_summary_chars.saturating_mul(comparable_sample_count)),
                    target_acc.saturating_add(
                        target_summary_chars.saturating_mul(comparable_sample_count),
                    ),
                )
            });
    compute_summary_char_growth_ratio(weighted_base_summary_chars, weighted_target_summary_chars)
}

fn format_optional_decimal(value: Option<f64>, decimals: usize) -> String {
    match value {
        Some(value) => format!("{value:.decimals$}"),
        None => "n/a".to_owned(),
    }
}

fn build_memory_context_soft_warnings(
    summary_window_cover_vs_window_only_ratio_p95: Option<f64>,
    summary_window_cover_overhead_p95_ms: Option<f64>,
    sample_count: usize,
    summary_window_cover_comparison_suite_is_noisy: bool,
    summary_rebuild_budget_change_vs_rebuild_ratio_p95: Option<f64>,
    summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95: Option<f64>,
    rebuild_budget_change_sample_count: usize,
    summary_metadata_realign_vs_budget_change_ratio_p95: Option<f64>,
    metadata_realign_sample_count: usize,
    speedup_ratio_suite_min: Option<f64>,
    speedup_ratio_suite_range_over_p50: Option<f64>,
    summary_rebuild_suite_range_over_p50: Option<f64>,
    summary_steady_state_suite_p50_ms: Option<f64>,
    summary_steady_state_suite_range_ms: Option<f64>,
    summary_steady_state_suite_range_over_p50: Option<f64>,
    suite_repetition_count: usize,
    normalized_min_speedup_ratio: f64,
    summary_rebuild_noise_attribution: Option<&MemoryContextColdPathNoiseAttribution>,
    summary_rebuild_target_bootstrap_noise_attribution: Option<
        &MemoryContextBootstrapNoiseAttribution,
    >,
    summary_rebuild_target_load_noise_attribution: Option<&MemoryContextLoadNoiseAttribution>,
    summary_rebuild_budget_change_suite_range_over_p50: Option<f64>,
    summary_metadata_realign_suite_range_over_p50: Option<f64>,
    summary_metadata_realign_vs_budget_change_ratio_suite_range_over_p50: Option<f64>,
    benchmark_temp_root_source: MemoryContextBenchmarkTempRootSource,
    benchmark_temp_root: &Path,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if matches!(
        benchmark_temp_root_source,
        MemoryContextBenchmarkTempRootSource::SystemTemp
    ) {
        warnings.push(format!(
            "benchmark_temp_root resolved to system temp {}; cold-path measurements can be noisy on OS-managed shared temp volumes, so prefer --temp-root or a target-dir-local tmp-local path for reproducible memory context benchmarks",
            benchmark_temp_root.display()
        ));
    }
    if suite_repetition_count >= DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_WARNING_MIN_SUITES {
        let speedup_ratio_suite_clear_win = speedup_ratio_suite_min.is_some_and(
            |speedup_ratio_suite_min| {
                speedup_ratio_suite_min
                    >= normalized_min_speedup_ratio
                        * DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_CLEAR_WIN_SUPPRESSION_MULTIPLIER
            },
        );
        let suppress_speedup_warning_for_clear_preload_noise_wins =
            summary_rebuild_suite_range_over_p50.is_some_and(|range_over_p50| {
                range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            }) && summary_rebuild_noise_attribution
                .is_some_and(|attribution| attribution.phase != "target_load_ms")
                && speedup_ratio_suite_clear_win;
        let suppress_speedup_warning_for_tiny_hot_path_denominator_jitter =
            summary_rebuild_suite_range_over_p50.is_some_and(|range_over_p50| {
                range_over_p50 <= DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            }) && summary_steady_state_suite_range_over_p50.is_some_and(|range_over_p50| {
                range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            }) && summary_steady_state_suite_p50_ms.is_some_and(|p50_ms| {
                p50_ms <= DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_TINY_HOT_PATH_MAX_P50_MS
            }) && summary_steady_state_suite_range_ms.is_some_and(|range_ms| {
                range_ms <= DEFAULT_MEMORY_CONTEXT_SPEEDUP_SUITE_NOISE_TINY_HOT_PATH_MAX_RANGE_MS
            }) && speedup_ratio_suite_clear_win;
        if let Some(range_over_p50) = speedup_ratio_suite_range_over_p50
            && range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            && !suppress_speedup_warning_for_clear_preload_noise_wins
            && !suppress_speedup_warning_for_tiny_hot_path_denominator_jitter
        {
            let attribution_suffix = match (
                summary_rebuild_suite_range_over_p50,
                summary_rebuild_noise_attribution,
            ) {
                (Some(summary_rebuild_range_over_p50), Some(attribution))
                    if summary_rebuild_range_over_p50
                        > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50 =>
                {
                    let phase_label = format_memory_context_cold_path_noise_phase_label(
                        attribution,
                        summary_rebuild_target_bootstrap_noise_attribution,
                        summary_rebuild_target_load_noise_attribution,
                    );
                    format!(
                        "; dominant summary_rebuild cold-path noise {} range_over_p50 {:.3}",
                        phase_label, attribution.range_over_p50
                    )
                }
                _ => String::new(),
            };
            if speedup_ratio_suite_clear_win {
                warnings.push(format!(
                    "speedup_ratio_p95 suite range_over_p50 {:.3} exceeded soft reproducibility threshold {:.3}; aggregated speedup is still a clear win and every suite still cleared the speedup floor by a wide margin, but the exact multiplier is host-sensitive, so rerun on a quieter host before over-interpreting the precise memory context speedup{}",
                    range_over_p50,
                    DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50,
                    attribution_suffix
                ));
            } else {
                warnings.push(format!(
                    "speedup_ratio_p95 suite range_over_p50 {:.3} exceeded soft reproducibility threshold {:.3}; aggregated speedup still reflects the median suite, but cross-suite spread is too large to trust small gains, so rerun on a quieter host before treating marginal memory context improvements as real{}",
                    range_over_p50,
                    DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50,
                    attribution_suffix
                ));
            }
        }
        if let Some(range_over_p50) = summary_rebuild_suite_range_over_p50
            && range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
        {
            let attribution_suffix = summary_rebuild_noise_attribution
                .map(|attribution| {
                    let phase_label = format_memory_context_cold_path_noise_phase_label(
                        attribution,
                        summary_rebuild_target_bootstrap_noise_attribution,
                        summary_rebuild_target_load_noise_attribution,
                    );
                    format!(
                        "; dominant cold-path phase {} range_over_p50 {:.3}",
                        phase_label, attribution.range_over_p50
                    )
                })
                .unwrap_or_default();
            warnings.push(format!(
                "summary_rebuild suite p95 range_over_p50 {:.3} exceeded soft reproducibility threshold {:.3}; cold-path rebuild cost is still host-noisy across suites, so inspect phase-level variance before over-interpreting one-off p95 wins{}",
                range_over_p50,
                DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50,
                attribution_suffix
            ));
        }
    }
    if sample_count >= DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_WARNING_MIN_SAMPLES
        && let (Some(ratio_p95), Some(overhead_p95_ms)) = (
            summary_window_cover_vs_window_only_ratio_p95,
            summary_window_cover_overhead_p95_ms,
        )
    {
        let marginal_cover_regression_under_suite_noise =
            summary_window_cover_comparison_suite_is_noisy
                && ratio_p95 <= DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_NOISY_SUPPRESSION_MAX_RATIO_P95
                && overhead_p95_ms
                    <= DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_NOISY_SUPPRESSION_MAX_OVERHEAD_P95_MS;
        if ratio_p95 > DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_RATIO_P95
            && overhead_p95_ms > DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_OVERHEAD_P95_MS
            && !marginal_cover_regression_under_suite_noise
        {
            if summary_window_cover_comparison_suite_is_noisy {
                warnings.push(format!(
                    "summary_window_cover p95 overhead {:.3}ms and ratio {:.3} exceeded soft thresholds {:.3}ms/{:.3}, but the cover-versus-window comparison is suite-noisy; rerun on a quieter host before treating the cover-path gap as actionable",
                    overhead_p95_ms,
                    ratio_p95,
                    DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_OVERHEAD_P95_MS,
                    DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_RATIO_P95
                ));
            } else {
                warnings.push(format!(
                    "summary_window_cover p95 overhead {:.3}ms and ratio {:.3} exceeded soft thresholds {:.3}ms/{:.3}; expected near-window-only cost when the active window already covers the session, so investigate redundant summary materialization or checkpoint work",
                    overhead_p95_ms,
                    ratio_p95,
                    DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_OVERHEAD_P95_MS,
                    DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_MAX_RATIO_P95
                ));
            }
        }
    }
    if rebuild_budget_change_sample_count
        >= DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_WARNING_MIN_SAMPLES
        && let (Some(raw_ratio_p95), Some(adjusted_ratio_p95)) = (
            summary_rebuild_budget_change_vs_rebuild_ratio_p95,
            summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95,
        )
        && adjusted_ratio_p95 > DEFAULT_MEMORY_CONTEXT_REBUILD_BUDGET_CHANGE_SOFT_MAX_RATIO_P95
    {
        warnings.push(format!(
            "summary_rebuild_budget_change raw p95 ratio {:.3} and summary-char-adjusted p95 ratio {:.3} exceeded soft threshold {:.3} versus full rebuild; expected metadata-first budget-change rebuild to scale with the larger rebuilt summary rather than regress beyond that workload, so investigate unnecessary checkpoint body loads or duplicate summary scans",
            raw_ratio_p95,
            adjusted_ratio_p95,
            DEFAULT_MEMORY_CONTEXT_REBUILD_BUDGET_CHANGE_SOFT_MAX_RATIO_P95
        ));
    }
    if metadata_realign_sample_count >= DEFAULT_MEMORY_CONTEXT_WINDOW_COVER_SOFT_WARNING_MIN_SAMPLES
        && let Some(ratio_p95) = summary_metadata_realign_vs_budget_change_ratio_p95
        && ratio_p95 > DEFAULT_MEMORY_CONTEXT_METADATA_REALIGN_SOFT_MAX_RATIO_P95
    {
        let suite_is_noisy =
            summary_rebuild_budget_change_suite_range_over_p50.is_some_and(|range_over_p50| {
                range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            }) || summary_metadata_realign_suite_range_over_p50.is_some_and(|range_over_p50| {
                range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
            }) || summary_metadata_realign_vs_budget_change_ratio_suite_range_over_p50.is_some_and(
                |range_over_p50| {
                    range_over_p50 > DEFAULT_MEMORY_CONTEXT_SUITE_STABILITY_SOFT_MAX_RANGE_OVER_P50
                },
            );
        if suite_is_noisy {
            warnings.push(format!(
                "summary_metadata_realign p95 ratio {:.3} exceeded soft threshold {:.3}, but the metadata-realign versus budget-change comparison is suite-noisy (ratio range_over_p50 {}, metadata {}, budget_change {}); rerun on a quieter host before attributing this to checkpoint-repair regressions",
                ratio_p95,
                DEFAULT_MEMORY_CONTEXT_METADATA_REALIGN_SOFT_MAX_RATIO_P95,
                format_optional_decimal(
                    summary_metadata_realign_vs_budget_change_ratio_suite_range_over_p50,
                    3
                ),
                format_optional_decimal(summary_metadata_realign_suite_range_over_p50, 3),
                format_optional_decimal(summary_rebuild_budget_change_suite_range_over_p50, 3),
            ));
        } else {
            warnings.push(format!(
                "summary_metadata_realign p95 ratio {:.3} exceeded soft threshold {:.3} versus budget-change rebuild; expected metadata-only checkpoint repair to stay no slower than budget-change rebuild, so investigate accidental summary body rewrites or redundant checkpoint updates",
                ratio_p95,
                DEFAULT_MEMORY_CONTEXT_METADATA_REALIGN_SOFT_MAX_RATIO_P95
            ));
        }
    }
    warnings
}

fn format_memory_context_cold_path_noise_phase_label(
    attribution: &MemoryContextColdPathNoiseAttribution,
    target_bootstrap_attribution: Option<&MemoryContextBootstrapNoiseAttribution>,
    target_load_attribution: Option<&MemoryContextLoadNoiseAttribution>,
) -> String {
    if attribution.phase == "target_bootstrap_ms"
        && let Some(target_bootstrap_attribution) = target_bootstrap_attribution
    {
        return format!("target_bootstrap_ms/{}", target_bootstrap_attribution.phase);
    }
    if attribution.phase == "target_load_ms"
        && let Some(target_load_attribution) = target_load_attribution
    {
        return format!("target_load_ms/{}", target_load_attribution.phase);
    }

    attribution.phase.clone()
}

fn memory_context_window_shrink_source_window(
    history_turns: usize,
    sliding_window: usize,
) -> CliResult<usize> {
    if history_turns <= sliding_window.saturating_add(1) {
        return Err(
            "history_turns must exceed sliding_window by at least 2 to exercise shrink catch-up mode"
                .to_owned(),
        );
    }

    Ok(sliding_window
        .saturating_mul(2)
        .min(history_turns.saturating_sub(1))
        .max(sliding_window.saturating_add(1)))
}

#[cfg(test)]
#[path = "memory_context_tests.rs"]
mod tests;
