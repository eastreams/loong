pub mod test_support;

#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH},
};

use kernel::{BridgeSupportMatrix, ChannelConfig, ConnectorCommand, ProviderConfig};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use tokio::time::{Instant as TokioInstant, sleep};

use loongclaw_spec::programmatic::{
    acquire_programmatic_circuit_slot, record_programmatic_circuit_outcome,
};
use loongclaw_spec::{
    BridgeRuntimePolicy, CliResult, ConnectorCircuitBreakerPolicy, NativeToolExecutor,
    ProgrammaticCircuitBreakerPolicy, ProgrammaticCircuitRuntimeState, RunnerSpec,
    execute_spec_with_native_tool_executor, execute_wasm_component_bridge,
    spec_requires_native_tool_executor,
};

const DEFAULT_PRESSURE_ITERATIONS: usize = 12;
const DEFAULT_PRESSURE_WARMUP_ITERATIONS: usize = 2;
const DEFAULT_CIRCUIT_POLL_INTERVAL_MS: u64 = 5;
const DEFAULT_CIRCUIT_RECOVERY_BUFFER_MS: u64 = 250;
const DEFAULT_WASM_CACHE_MIN_SPEEDUP_RATIO: f64 = 1.5;
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
const BENCHMARK_COPY_STRATEGY_ENV: &str = "LOONGCLAW_BENCHMARK_COPY_STRATEGY";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BenchmarkCopyStrategy {
    StableFsCopy,
    #[cfg(target_os = "macos")]
    MacosCloneCp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgrammaticPressureMatrix {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    baseline_path: Option<String>,
    #[serde(default = "default_pressure_iterations")]
    default_iterations: usize,
    #[serde(default = "default_pressure_warmup_iterations")]
    default_warmup_iterations: usize,
    scenarios: Vec<ProgrammaticPressureScenario>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgrammaticPressureScenario {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub iterations: Option<usize>,
    #[serde(default)]
    pub warmup_iterations: Option<usize>,
    #[serde(default = "default_pressure_expected_operation_kind")]
    pub expected_operation_kind: String,
    #[serde(default)]
    pub allow_blocked: bool,
    #[serde(flatten)]
    pub kind: ProgrammaticPressureScenarioKind,
}

#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ProgrammaticPressureScenarioKind {
    SpecRun {
        spec: RunnerSpec,
    },
    CircuitHalfOpen {
        connector_name: String,
        policy: ProgrammaticCircuitBreakerPolicy,
        #[serde(default = "default_failures_before_open")]
        failures_before_open: usize,
        #[serde(default)]
        recovery_successes: Option<usize>,
        #[serde(default)]
        poll_interval_ms: Option<u64>,
        #[serde(default)]
        cooldown_poll_timeout_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProgrammaticPressureBaseline {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    scenarios: BTreeMap<String, ProgrammaticPressureScenarioThresholds>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProgrammaticPressureScenarioThresholds {
    #[serde(default)]
    max_error_rate: Option<f64>,
    #[serde(default)]
    max_p95_latency_ms: Option<f64>,
    #[serde(default)]
    max_p99_latency_ms: Option<f64>,
    #[serde(default)]
    min_throughput_rps: Option<f64>,
    #[serde(default)]
    min_peak_in_flight: Option<f64>,
    #[serde(default)]
    max_circuit_open_error_ratio: Option<f64>,
    #[serde(default)]
    max_half_open_p95_ms: Option<f64>,
    #[serde(default)]
    expected_schema_fingerprint: Option<String>,
    #[serde(default)]
    tolerance: ProgrammaticPressureGateTolerance,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProgrammaticPressureGateTolerance {
    #[serde(default)]
    max_ratio: f64,
    #[serde(default)]
    min_ratio: f64,
    #[serde(default)]
    latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureReport {
    generated_at_epoch_s: u64,
    profile: Option<String>,
    matrix_path: String,
    baseline_path: Option<String>,
    baseline_profile: Option<String>,
    scenario_count: usize,
    scenarios: Vec<ProgrammaticPressureScenarioReport>,
    gate: ProgrammaticPressureGateSummary,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureScenarioReport {
    name: String,
    description: Option<String>,
    scenario_kind: String,
    iterations: usize,
    warmup_iterations: usize,
    success_runs: usize,
    failed_runs: usize,
    blocked_runs: usize,
    error_rate: f64,
    blocked_rate: f64,
    connector_calls_total: usize,
    throughput_rps: f64,
    latency_ms: NumericStats,
    circuit_open_error_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_fingerprint: Option<String>,
    scheduler: Option<ProgrammaticSchedulerAggregate>,
    circuit: Option<ProgrammaticCircuitAggregate>,
    error_codes: BTreeMap<String, usize>,
    gate: ProgrammaticPressureScenarioGate,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticSchedulerAggregate {
    observed_runs: usize,
    peak_in_flight_max: usize,
    peak_in_flight_avg: f64,
    budget_reductions_total: usize,
    budget_increases_total: usize,
    wait_cycles_total: usize,
    min_final_in_flight_budget: usize,
    max_final_in_flight_budget: usize,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticCircuitAggregate {
    observed_runs: usize,
    half_open_transition_ms: NumericStats,
    closed_after_recovery_rate: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
struct NumericStats {
    count: usize,
    min: Option<f64>,
    max: Option<f64>,
    avg: Option<f64>,
    p50: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureGateSummary {
    enforced: bool,
    passed: bool,
    failed_scenarios: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preflight: Option<ProgrammaticPressureBaselinePreflight>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureBaselinePreflight {
    strict: bool,
    fail_on_warnings: bool,
    passed: bool,
    gate_passed: bool,
    error_count: usize,
    warning_count: usize,
    issue_count: usize,
    issues: Vec<ProgrammaticPressureBaselineIssue>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProgrammaticPressureBaselineIssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProgrammaticPressureBaselineIssueKind {
    DuplicateMatrixScenarioName,
    MissingMatrixScenarioBaselineThresholds,
    MissingSpecRunBaselineScenario,
    MissingSpecRunSchemaFingerprint,
    UnknownBaselineScenario,
    NonSpecRunSchemaFingerprintConfigured,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ProgrammaticPressureBaselineIssue {
    severity: ProgrammaticPressureBaselineIssueSeverity,
    kind: ProgrammaticPressureBaselineIssueKind,
    scenario_name: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureBaselineLintReport {
    generated_at_epoch_s: u64,
    matrix_path: String,
    baseline_path: String,
    profile: Option<String>,
    baseline_profile: Option<String>,
    scenario_count: usize,
    spec_run_scenario_count: usize,
    baseline_scenario_count: usize,
    fail_on_warnings: bool,
    passed: bool,
    gate_passed: bool,
    error_count: usize,
    warning_count: usize,
    issues: Vec<ProgrammaticPressureBaselineIssue>,
}

#[derive(Debug, Clone, Serialize)]
struct WasmCacheBenchmarkReport {
    generated_at_epoch_s: u64,
    profile: String,
    input_wasm_path: String,
    effective_wasm_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_note: Option<String>,
    output_path: String,
    cold_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    cold_latency_ms: NumericStats,
    hot_latency_ms: NumericStats,
    cold_cache_hits: usize,
    cold_cache_misses: usize,
    hot_cache_hits: usize,
    hot_cache_misses: usize,
    speedup_ratio_p95: Option<f64>,
    gate: WasmCacheBenchmarkGateSummary,
}

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
    flattened_sample_ratios: MemoryContextRatioP95Summary,
    aggregated_p95_median_ms: MemoryContextAggregatedP95MedianMs,
    aggregated_ratios: MemoryContextRatioP95Summary,
    gate: MemoryContextBenchmarkGateSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MemoryContextBenchmarkTempRootSource {
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
struct ResolvedMemoryContextBenchmarkTempRoot {
    path: PathBuf,
    source: MemoryContextBenchmarkTempRootSource,
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
struct WasmCacheBenchmarkGateSummary {
    enforced: bool,
    passed: bool,
    min_speedup_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_speedup_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureScenarioGate {
    passed: bool,
    checks: Vec<ProgrammaticPressureGateCheck>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticPressureGateCheck {
    metric: String,
    comparator: String,
    threshold: f64,
    observed: f64,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[doc(hidden)]
pub struct ScenarioRunSample {
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
struct SchedulerSnapshot {
    peak_in_flight: usize,
    final_in_flight_budget: usize,
    budget_reductions: usize,
    budget_increases: usize,
    wait_cycles: usize,
}

#[derive(Debug, Clone, Copy)]
struct WasmBridgeSample {
    latency_ms: f64,
    cache_hit: bool,
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

#[cfg(test)]
#[derive(Debug, Clone)]
struct PromptContextReadObservation {
    latency_ms: f64,
    rss_delta_kib: Option<f64>,
    shape: MemoryContextShape,
}

#[cfg(test)]
fn measure_hot_prompt_context_reads_with_loader(
    warmup_iterations: usize,
    hot_iterations: usize,
    expect_summary: bool,
    mut load_observation: impl FnMut() -> CliResult<PromptContextReadObservation>,
) -> CliResult<(Vec<f64>, Vec<f64>, MemoryContextShape)> {
    for _ in 0..warmup_iterations.max(1) {
        let observation = load_observation()?;
        validate_prompt_context_shape(observation.shape, expect_summary, "warmup")?;
    }

    let mut latencies = Vec::with_capacity(hot_iterations);
    let mut rss_deltas_kib = Vec::with_capacity(hot_iterations);
    let mut final_shape = MemoryContextShape {
        entry_count: 0,
        turn_entries: 0,
        summary_chars: 0,
        payload_chars: 0,
    };

    for _ in 0..hot_iterations {
        let observation = load_observation()?;
        latencies.push(observation.latency_ms);
        if let Some(delta_kib) = observation.rss_delta_kib {
            rss_deltas_kib.push(delta_kib);
        }
        validate_prompt_context_shape(observation.shape, expect_summary, "sample")?;
        final_shape = observation.shape;
    }

    Ok((latencies, rss_deltas_kib, final_shape))
}

#[cfg(test)]
fn validate_prompt_context_shape(
    shape: MemoryContextShape,
    expect_summary: bool,
    phase: &str,
) -> CliResult<()> {
    if expect_summary && shape.summary_chars == 0 {
        return Err(format!(
            "summary benchmark {phase} did not produce a summary entry"
        ));
    }
    if !expect_summary && shape.summary_chars != 0 {
        return Err(format!(
            "window-only benchmark {phase} unexpectedly produced a summary entry"
        ));
    }
    Ok(())
}

#[cfg(test)]
fn parse_ps_rss_kib_output(raw: &str) -> Option<f64> {
    let token = raw.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.split_whitespace().next()
        }
    })?;
    token.parse::<f64>().ok()
}

#[cfg(test)]
fn compute_rss_step_delta_kib(baseline_kib: Option<f64>, current_kib: Option<f64>) -> Option<f64> {
    let baseline_kib = baseline_kib?;
    let current_kib = current_kib?;
    Some((current_kib - baseline_kib).max(0.0))
}

#[allow(clippy::print_stdout)] // CLI benchmark report output
pub async fn run_programmatic_pressure_benchmark_cli(
    matrix_path: &str,
    baseline_path: Option<&str>,
    output_path: &str,
    enforce_gate: bool,
    preflight_fail_on_warnings: bool,
    native_tool_executor: Option<NativeToolExecutor>,
) -> CliResult<()> {
    let matrix: ProgrammaticPressureMatrix = read_json_file(matrix_path)?;

    let selected_baseline_path = baseline_path
        .map(str::to_owned)
        .or_else(|| matrix.baseline_path.clone());
    if enforce_gate && selected_baseline_path.is_none() {
        return Err(
            "benchmark gate enforcement requires --baseline or matrix.baseline_path".to_owned(),
        );
    }

    let baseline = if let Some(path) = selected_baseline_path.as_deref() {
        Some(read_json_file::<ProgrammaticPressureBaseline>(path)?)
    } else {
        None
    };
    let baseline_lint = baseline
        .as_ref()
        .map(|value| lint_programmatic_pressure_baseline(&matrix, value));
    let preflight = baseline_lint
        .as_ref()
        .map(|lint| build_baseline_preflight(lint, enforce_gate, preflight_fail_on_warnings));
    if enforce_gate
        && let Some(preflight_summary) = preflight.as_ref()
        && !preflight_summary.gate_passed
    {
        let gate_issues = preflight_gate_issues(
            &preflight_summary.issues,
            preflight_summary.fail_on_warnings,
        );
        return Err(format!(
            "programmatic pressure strict preflight failed: {}",
            format_baseline_issue_list(&gate_issues)
        ));
    }

    let report = run_programmatic_pressure_matrix(
        &matrix,
        matrix_path,
        selected_baseline_path.as_deref(),
        baseline.as_ref(),
        preflight,
        enforce_gate,
        native_tool_executor,
    )
    .await;

    write_json_file(output_path, &report)?;

    println!("programmatic pressure benchmark report written to {output_path}");
    for scenario in &report.scenarios {
        let p95 = scenario.latency_ms.p95.unwrap_or(0.0);
        println!(
            "scenario={} kind={} pass={}/{} p95_ms={:.2} throughput_rps={:.2} gate={}",
            scenario.name,
            scenario.scenario_kind,
            scenario.success_runs,
            scenario.iterations,
            p95,
            scenario.throughput_rps,
            if scenario.gate.passed { "pass" } else { "fail" }
        );
    }

    if report.gate.passed {
        println!("benchmark gate status: passed");
        Ok(())
    } else {
        println!("benchmark gate status: failed");
        if enforce_gate {
            Err(format!(
                "programmatic pressure benchmark regression gate failed for scenarios: {}",
                report.gate.failed_scenarios.join(", ")
            ))
        } else {
            Ok(())
        }
    }
}

#[allow(clippy::print_stdout)] // CLI lint report output
pub fn run_programmatic_pressure_baseline_lint_cli(
    matrix_path: &str,
    baseline_path: Option<&str>,
    output_path: &str,
    enforce_gate: bool,
    fail_on_warnings: bool,
) -> CliResult<()> {
    let matrix: ProgrammaticPressureMatrix = read_json_file(matrix_path)?;
    let selected_baseline_path = baseline_path
        .map(str::to_owned)
        .or_else(|| matrix.baseline_path.clone());
    let baseline_path = selected_baseline_path.ok_or_else(|| {
        "programmatic pressure baseline lint requires --baseline or matrix.baseline_path".to_owned()
    })?;
    let baseline: ProgrammaticPressureBaseline = read_json_file(&baseline_path)?;
    let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
    let gate_passed = lint_gate_passed(&lint, fail_on_warnings);

    let report = ProgrammaticPressureBaselineLintReport {
        generated_at_epoch_s: current_epoch_seconds(),
        matrix_path: matrix_path.to_owned(),
        baseline_path,
        profile: matrix.profile.clone(),
        baseline_profile: baseline.profile.clone(),
        scenario_count: matrix.scenarios.len(),
        spec_run_scenario_count: matrix
            .scenarios
            .iter()
            .filter(|scenario| {
                matches!(
                    scenario.kind,
                    ProgrammaticPressureScenarioKind::SpecRun { .. }
                )
            })
            .count(),
        baseline_scenario_count: baseline.scenarios.len(),
        fail_on_warnings,
        passed: lint.passed(),
        gate_passed,
        error_count: lint.error_count(),
        warning_count: lint.warning_count(),
        issues: lint.issues,
    };

    write_json_file(output_path, &report)?;
    println!("programmatic pressure baseline lint report written to {output_path}");
    println!(
        "baseline lint: passed={} gate_passed={} errors={} warnings={} fail_on_warnings={}",
        report.passed,
        report.gate_passed,
        report.error_count,
        report.warning_count,
        report.fail_on_warnings
    );
    for issue in &report.issues {
        println!(
            "issue[{:?}/{:?}] scenario={} {}",
            issue.severity, issue.kind, issue.scenario_name, issue.message
        );
    }

    if enforce_gate && !report.gate_passed {
        let gate_issues = preflight_gate_issues(&report.issues, fail_on_warnings);
        return Err(format!(
            "programmatic pressure baseline lint failed: {}",
            format_baseline_issue_list(&gate_issues)
        ));
    }

    Ok(())
}

#[allow(clippy::print_stdout)] // CLI benchmark report output
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

    write_json_file(output_path, &report)?;
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

#[allow(clippy::print_stdout)] // CLI benchmark report output
pub fn run_wasm_cache_benchmark_cli(
    wasm_path: &str,
    output_path: &str,
    cold_iterations: usize,
    hot_iterations: usize,
    warmup_iterations: usize,
    enforce_gate: bool,
    min_speedup_ratio: f64,
) -> CliResult<()> {
    if cold_iterations == 0 {
        return Err("wasm cache benchmark requires cold_iterations > 0".to_owned());
    }
    if hot_iterations == 0 {
        return Err("wasm cache benchmark requires hot_iterations > 0".to_owned());
    }

    let normalized_min_speedup_ratio = if min_speedup_ratio.is_finite() && min_speedup_ratio > 0.0 {
        min_speedup_ratio
    } else {
        DEFAULT_WASM_CACHE_MIN_SPEEDUP_RATIO
    };

    let wasm_source = Path::new(wasm_path);
    if !wasm_source.exists() {
        return Err(format!("wasm artifact does not exist: {wasm_path}"));
    }
    let wasm_source = fs::canonicalize(wasm_source)
        .map_err(|error| format!("failed to canonicalize wasm artifact path: {error}"))?;
    let temp_root = std::env::temp_dir().join(format!(
        "loongclaw-wasm-cache-benchmark-{}",
        current_epoch_seconds()
    ));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create benchmark temp directory: {error}"))?;

    let (benchmark_source, source_note) = {
        let metadata = fs::metadata(&wasm_source)
            .map_err(|error| format!("failed to read wasm metadata for benchmark: {error}"))?;
        if metadata.len() <= 8 {
            let synthetic_source = temp_root.join("synthetic-benchmark-module.wasm");
            write_synthetic_wasm_benchmark_module(&synthetic_source)?;
            (
                synthetic_source,
                Some(format!(
                    "input wasm `{}` appears to be a placeholder ({} bytes); using synthetic benchmark module with exported `run` function",
                    wasm_source.display(),
                    metadata.len()
                )),
            )
        } else {
            (wasm_source.clone(), None)
        }
    };

    let mut cold_latencies_ms = Vec::with_capacity(cold_iterations);
    let mut cold_cache_hits = 0usize;
    let mut cold_cache_misses = 0usize;
    for iteration in 0..cold_iterations {
        let candidate = temp_root.join(format!("cold-{iteration}.wasm"));
        fs::copy(&benchmark_source, &candidate)
            .map_err(|error| format!("failed to prepare cold benchmark artifact: {error}"))?;
        let sample = run_wasm_bridge_sample(&candidate)?;
        cold_latencies_ms.push(sample.latency_ms);
        if sample.cache_hit {
            cold_cache_hits = cold_cache_hits.saturating_add(1);
        } else {
            cold_cache_misses = cold_cache_misses.saturating_add(1);
        }
    }

    let hot_artifact = temp_root.join("hot.wasm");
    fs::copy(&benchmark_source, &hot_artifact)
        .map_err(|error| format!("failed to prepare hot benchmark artifact: {error}"))?;
    for _ in 0..warmup_iterations {
        let _ = run_wasm_bridge_sample(&hot_artifact)?;
    }

    let mut hot_latencies_ms = Vec::with_capacity(hot_iterations);
    let mut hot_cache_hits = 0usize;
    let mut hot_cache_misses = 0usize;
    for _ in 0..hot_iterations {
        let sample = run_wasm_bridge_sample(&hot_artifact)?;
        hot_latencies_ms.push(sample.latency_ms);
        if sample.cache_hit {
            hot_cache_hits = hot_cache_hits.saturating_add(1);
        } else {
            hot_cache_misses = hot_cache_misses.saturating_add(1);
        }
    }

    let cold_latency_ms = compute_numeric_stats(&cold_latencies_ms);
    let hot_latency_ms = compute_numeric_stats(&hot_latencies_ms);
    let observed_speedup_ratio = match (cold_latency_ms.p95, hot_latency_ms.p95) {
        (Some(cold_p95), Some(hot_p95)) if hot_p95 > 0.0 => Some(cold_p95 / hot_p95),
        _ => None,
    };

    let mut gate_reason = None;
    let gate_passed = if enforce_gate {
        match observed_speedup_ratio {
            Some(observed) if observed >= normalized_min_speedup_ratio => true,
            Some(observed) => {
                gate_reason = Some(format!(
                    "observed p95 speedup ratio {:.3} is below threshold {:.3}",
                    observed, normalized_min_speedup_ratio
                ));
                false
            }
            None => {
                gate_reason = Some("unable to compute p95 speedup ratio".to_owned());
                false
            }
        }
    } else {
        true
    };

    let report = WasmCacheBenchmarkReport {
        generated_at_epoch_s: current_epoch_seconds(),
        profile: "release".to_owned(),
        input_wasm_path: wasm_source.display().to_string(),
        effective_wasm_path: benchmark_source.display().to_string(),
        source_note: source_note.clone(),
        output_path: output_path.to_owned(),
        cold_iterations,
        hot_iterations,
        warmup_iterations,
        cold_latency_ms,
        hot_latency_ms,
        cold_cache_hits,
        cold_cache_misses,
        hot_cache_hits,
        hot_cache_misses,
        speedup_ratio_p95: observed_speedup_ratio,
        gate: WasmCacheBenchmarkGateSummary {
            enforced: enforce_gate,
            passed: gate_passed,
            min_speedup_ratio: normalized_min_speedup_ratio,
            observed_speedup_ratio,
            reason: gate_reason.clone(),
        },
    };

    write_json_file(output_path, &report)?;
    println!("wasm cache benchmark report written to {output_path}");
    println!(
        "cold p95={:.3}ms hot p95={:.3}ms speedup_ratio_p95={:.3} gate={}",
        report.cold_latency_ms.p95.unwrap_or(0.0),
        report.hot_latency_ms.p95.unwrap_or(0.0),
        report.speedup_ratio_p95.unwrap_or(0.0),
        if report.gate.passed { "pass" } else { "fail" }
    );
    println!(
        "cache cold(hit/miss)={}/{} hot(hit/miss)={}/{}",
        report.cold_cache_hits,
        report.cold_cache_misses,
        report.hot_cache_hits,
        report.hot_cache_misses
    );
    if let Some(note) = source_note {
        println!("note: {note}");
    }

    let _ = fs::remove_dir_all(&temp_root);

    if enforce_gate && !gate_passed {
        let reason = gate_reason.unwrap_or_else(|| "gate failed".to_owned());
        return Err(format!(
            "wasm cache benchmark regression gate failed: {reason}"
        ));
    }

    Ok(())
}

fn write_synthetic_wasm_benchmark_module(path: &Path) -> CliResult<()> {
    // Minimal wasm module exporting empty function `run`.
    const SYNTHETIC_WASM_WITH_RUN_EXPORT: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, // magic
        0x01, 0x00, 0x00, 0x00, // version
        0x01, 0x04, 0x01, 0x60, 0x00, 0x00, // type section
        0x03, 0x02, 0x01, 0x00, // function section
        0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x00, // export section
        0x0a, 0x04, 0x01, 0x02, 0x00, 0x0b, // code section
    ];
    fs::write(path, SYNTHETIC_WASM_WITH_RUN_EXPORT)
        .map_err(|error| format!("failed to write synthetic wasm benchmark module: {error}"))?;
    Ok(())
}

fn run_wasm_bridge_sample(wasm_artifact: &Path) -> CliResult<WasmBridgeSample> {
    let canonical_artifact = fs::canonicalize(wasm_artifact).map_err(|error| {
        format!(
            "failed to canonicalize benchmark wasm artifact {}: {error}",
            wasm_artifact.display()
        )
    })?;
    let artifact_parent = canonical_artifact.parent().ok_or_else(|| {
        format!(
            "failed to compute parent directory for {}",
            canonical_artifact.display()
        )
    })?;
    let provider = ProviderConfig {
        provider_id: "wasm-cache-benchmark-provider".to_owned(),
        connector_name: "wasm-cache-benchmark-provider".to_owned(),
        version: "0.1.0".to_owned(),
        metadata: BTreeMap::from([
            (
                "component_resolved_path".to_owned(),
                canonical_artifact.display().to_string(),
            ),
            ("entrypoint".to_owned(), "run".to_owned()),
        ]),
    };
    let channel = ChannelConfig {
        channel_id: "primary".to_owned(),
        provider_id: provider.provider_id.clone(),
        endpoint: canonical_artifact.display().to_string(),
        enabled: true,
        metadata: BTreeMap::new(),
    };
    let command = ConnectorCommand {
        connector_name: provider.connector_name.clone(),
        operation: "invoke".to_owned(),
        required_capabilities: BTreeSet::new(),
        payload: json!({"benchmark":"wasm_cache"}),
    };
    let runtime_policy = BridgeRuntimePolicy {
        execute_process_stdio: false,
        execute_http_json: false,
        execute_wasm_component: true,
        compatibility_matrix: BridgeSupportMatrix::default(),
        allowed_process_commands: BTreeSet::new(),
        bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
        wasm_allowed_path_prefixes: vec![artifact_parent.to_path_buf()],
        wasm_max_component_bytes: Some(8 * 1024 * 1024),
        wasm_max_output_bytes: None,
        wasm_fuel_limit: Some(2_000_000),
        wasm_timeout_ms: None,
        wasm_require_hash_pin: false,
        wasm_required_sha256_by_plugin: BTreeMap::new(),
        enforce_execution_success: true,
    };

    let started_at = StdInstant::now();
    let execution =
        execute_wasm_component_bridge(json!({}), &provider, &channel, &command, &runtime_policy);
    let latency_ms = started_at.elapsed().as_secs_f64() * 1_000.0;
    let status = execution
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status != "executed" {
        let reason = execution
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown reason");
        return Err(format!(
            "wasm bridge benchmark execution failed for {}: status={status} reason={reason}",
            canonical_artifact.display()
        ));
    }

    let cache_hit = execution
        .get("runtime")
        .and_then(Value::as_object)
        .and_then(|runtime| runtime.get("cache_hit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(WasmBridgeSample {
        latency_ms,
        cache_hit,
    })
}

#[doc(hidden)]
pub fn copy_benchmark_file(source: &Path, destination: &Path) -> CliResult<()> {
    match benchmark_copy_strategy_from_env(std::env::var(BENCHMARK_COPY_STRATEGY_ENV).ok()) {
        #[cfg(target_os = "macos")]
        BenchmarkCopyStrategy::MacosCloneCp => {
            let clone_attempt = Command::new("/bin/cp")
                .arg("-c")
                .arg(source)
                .arg(destination)
                .output();
            if let Ok(output) = clone_attempt
                && output.status.success()
            {
                return Ok(());
            }

            if destination.exists() {
                let _ = fs::remove_file(destination);
            }
        }
        BenchmarkCopyStrategy::StableFsCopy => {}
    }

    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "copy benchmark file {} -> {} failed: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn benchmark_copy_strategy_from_env(_raw: Option<String>) -> BenchmarkCopyStrategy {
    #[cfg(target_os = "macos")]
    {
        if _raw
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("clone"))
        {
            return BenchmarkCopyStrategy::MacosCloneCp;
        }
    }

    BenchmarkCopyStrategy::StableFsCopy
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

async fn run_programmatic_pressure_matrix(
    matrix: &ProgrammaticPressureMatrix,
    matrix_path: &str,
    baseline_path: Option<&str>,
    baseline: Option<&ProgrammaticPressureBaseline>,
    preflight: Option<ProgrammaticPressureBaselinePreflight>,
    enforce_gate: bool,
    native_tool_executor: Option<NativeToolExecutor>,
) -> ProgrammaticPressureReport {
    let mut scenarios = Vec::new();
    for scenario in &matrix.scenarios {
        let baseline_thresholds = baseline.and_then(|value| value.scenarios.get(&scenario.name));
        let report = run_programmatic_pressure_scenario(
            matrix,
            scenario,
            baseline_thresholds,
            enforce_gate,
            native_tool_executor,
        )
        .await;
        scenarios.push(report);
    }

    let failed_scenarios: Vec<String> = scenarios
        .iter()
        .filter(|scenario| !scenario.gate.passed)
        .map(|scenario| scenario.name.clone())
        .collect();

    ProgrammaticPressureReport {
        generated_at_epoch_s: current_epoch_seconds(),
        profile: matrix.profile.clone(),
        matrix_path: matrix_path.to_owned(),
        baseline_path: baseline_path.map(str::to_owned),
        baseline_profile: baseline.and_then(|value| value.profile.clone()),
        scenario_count: scenarios.len(),
        scenarios,
        gate: ProgrammaticPressureGateSummary {
            enforced: enforce_gate,
            passed: failed_scenarios.is_empty(),
            failed_scenarios,
            preflight,
        },
    }
}

#[derive(Debug, Clone, Default)]
struct ProgrammaticPressureBaselineLintResult {
    issues: Vec<ProgrammaticPressureBaselineIssue>,
}

impl ProgrammaticPressureBaselineLintResult {
    fn passed(&self) -> bool {
        self.error_count() == 0
    }

    fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ProgrammaticPressureBaselineIssueSeverity::Error)
            .count()
    }

    fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == ProgrammaticPressureBaselineIssueSeverity::Warning)
            .count()
    }
}

fn build_baseline_preflight(
    lint: &ProgrammaticPressureBaselineLintResult,
    strict: bool,
    fail_on_warnings: bool,
) -> ProgrammaticPressureBaselinePreflight {
    ProgrammaticPressureBaselinePreflight {
        strict,
        fail_on_warnings,
        passed: lint.passed(),
        gate_passed: lint_gate_passed(lint, fail_on_warnings),
        error_count: lint.error_count(),
        warning_count: lint.warning_count(),
        issue_count: lint.issues.len(),
        issues: lint.issues.clone(),
    }
}

fn preflight_gate_issues(
    issues: &[ProgrammaticPressureBaselineIssue],
    fail_on_warnings: bool,
) -> Vec<ProgrammaticPressureBaselineIssue> {
    issues
        .iter()
        .filter(|issue| match issue.severity {
            ProgrammaticPressureBaselineIssueSeverity::Error => true,
            ProgrammaticPressureBaselineIssueSeverity::Warning => fail_on_warnings,
        })
        .cloned()
        .collect()
}

fn lint_gate_passed(lint: &ProgrammaticPressureBaselineLintResult, fail_on_warnings: bool) -> bool {
    if lint.error_count() > 0 {
        return false;
    }
    if fail_on_warnings && lint.warning_count() > 0 {
        return false;
    }
    true
}

fn lint_programmatic_pressure_baseline(
    matrix: &ProgrammaticPressureMatrix,
    baseline: &ProgrammaticPressureBaseline,
) -> ProgrammaticPressureBaselineLintResult {
    let mut issues = Vec::new();
    let mut matrix_name_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for scenario in &matrix.scenarios {
        *matrix_name_counts
            .entry(scenario.name.as_str())
            .or_insert(0) += 1;
    }
    for (scenario_name, count) in &matrix_name_counts {
        if *count > 1 {
            issues.push(ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Error,
                kind: ProgrammaticPressureBaselineIssueKind::DuplicateMatrixScenarioName,
                scenario_name: (*scenario_name).to_owned(),
                message: format!(
                    "matrix defines duplicate scenario name {scenario_name} ({count} entries)"
                ),
            });
        }
    }

    let matrix_scenario_map: BTreeMap<&str, &ProgrammaticPressureScenario> = matrix
        .scenarios
        .iter()
        .map(|scenario| (scenario.name.as_str(), scenario))
        .collect();

    for scenario in &matrix.scenarios {
        let is_spec_run = matches!(
            &scenario.kind,
            ProgrammaticPressureScenarioKind::SpecRun { .. }
        );
        match baseline.scenarios.get(&scenario.name) {
            Some(thresholds) => {
                if is_spec_run
                    && !baseline_has_schema_fingerprint(
                        thresholds.expected_schema_fingerprint.as_deref(),
                    )
                {
                    issues.push(ProgrammaticPressureBaselineIssue {
                        severity: ProgrammaticPressureBaselineIssueSeverity::Error,
                        kind:
                            ProgrammaticPressureBaselineIssueKind::MissingSpecRunSchemaFingerprint,
                        scenario_name: scenario.name.clone(),
                        message:
                            "expected_schema_fingerprint missing for spec_run scenario in baseline"
                                .to_owned(),
                    });
                }
                if !is_spec_run
                    && baseline_has_schema_fingerprint(
                        thresholds.expected_schema_fingerprint.as_deref(),
                    )
                {
                    issues.push(ProgrammaticPressureBaselineIssue {
                        severity: ProgrammaticPressureBaselineIssueSeverity::Warning,
                        kind: ProgrammaticPressureBaselineIssueKind::NonSpecRunSchemaFingerprintConfigured,
                        scenario_name: scenario.name.clone(),
                        message: "expected_schema_fingerprint is ignored for non-spec_run scenario"
                            .to_owned(),
                    });
                }
            }
            None if is_spec_run => issues.push(ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Error,
                kind: ProgrammaticPressureBaselineIssueKind::MissingSpecRunBaselineScenario,
                scenario_name: scenario.name.clone(),
                message: "baseline scenario missing for spec_run coverage".to_owned(),
            }),
            None => issues.push(ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Error,
                kind:
                    ProgrammaticPressureBaselineIssueKind::MissingMatrixScenarioBaselineThresholds,
                scenario_name: scenario.name.clone(),
                message: "baseline scenario missing for matrix scenario coverage".to_owned(),
            }),
        }
    }

    for scenario_name in baseline.scenarios.keys() {
        let matrix_scenario = matrix_scenario_map.get(scenario_name.as_str());
        if matrix_scenario.is_none() {
            issues.push(ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Warning,
                kind: ProgrammaticPressureBaselineIssueKind::UnknownBaselineScenario,
                scenario_name: scenario_name.clone(),
                message: "baseline scenario does not exist in matrix".to_owned(),
            });
            continue;
        }
    }

    ProgrammaticPressureBaselineLintResult { issues }
}

fn format_baseline_issue_list(issues: &[ProgrammaticPressureBaselineIssue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{} ({})", issue.scenario_name, issue.message))
        .collect::<Vec<_>>()
        .join(", ")
}

fn baseline_has_schema_fingerprint(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .map(|fingerprint| !fingerprint.is_empty())
        .unwrap_or(false)
}

async fn run_programmatic_pressure_scenario(
    matrix: &ProgrammaticPressureMatrix,
    scenario: &ProgrammaticPressureScenario,
    thresholds: Option<&ProgrammaticPressureScenarioThresholds>,
    enforce_gate: bool,
    native_tool_executor: Option<NativeToolExecutor>,
) -> ProgrammaticPressureScenarioReport {
    let iterations = scenario
        .iterations
        .unwrap_or(matrix.default_iterations)
        .max(1);
    let warmup_iterations = scenario
        .warmup_iterations
        .unwrap_or(matrix.default_warmup_iterations);

    for _ in 0..warmup_iterations {
        let _ = run_pressure_scenario_once(scenario, native_tool_executor).await;
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = TokioInstant::now();
        let run = run_pressure_scenario_once(scenario, native_tool_executor).await;
        let latency_ms = started.elapsed().as_secs_f64() * 1_000.0;
        samples.push(match run {
            Ok(mut sample) => {
                sample.latency_ms = latency_ms;
                sample
            }
            Err(error) => {
                let mut sample = ScenarioRunSample {
                    latency_ms,
                    passed: false,
                    ..ScenarioRunSample::default()
                };
                let code = parse_programmatic_error_code(&error)
                    .unwrap_or_else(|| "scenario_runtime_error".to_owned());
                increment_error_code(&mut sample.error_codes, &code);
                sample
            }
        });
    }

    summarize_programmatic_pressure_scenario(
        scenario,
        iterations,
        warmup_iterations,
        samples,
        thresholds,
        enforce_gate,
    )
}

async fn run_pressure_scenario_once(
    scenario: &ProgrammaticPressureScenario,
    native_tool_executor: Option<NativeToolExecutor>,
) -> CliResult<ScenarioRunSample> {
    match &scenario.kind {
        ProgrammaticPressureScenarioKind::SpecRun { spec } => {
            run_spec_pressure_once(spec, scenario, native_tool_executor).await
        }
        ProgrammaticPressureScenarioKind::CircuitHalfOpen {
            connector_name,
            policy,
            failures_before_open,
            recovery_successes,
            poll_interval_ms,
            cooldown_poll_timeout_ms,
        } => {
            run_circuit_half_open_pressure_once(
                connector_name,
                policy,
                *failures_before_open,
                recovery_successes.unwrap_or(policy.success_threshold),
                *poll_interval_ms,
                *cooldown_poll_timeout_ms,
            )
            .await
        }
    }
}

#[doc(hidden)]
pub async fn run_spec_pressure_once(
    spec: &RunnerSpec,
    scenario: &ProgrammaticPressureScenario,
    native_tool_executor: Option<NativeToolExecutor>,
) -> CliResult<ScenarioRunSample> {
    let requires_native_tool_executor = spec_requires_native_tool_executor(spec);
    if requires_native_tool_executor && native_tool_executor.is_none() {
        return Err(
            "spec benchmark scenario requires a native tool executor; move this claw.migrate/claw_migrate run to daemon composition root".to_owned(),
        );
    }
    let report = execute_spec_with_native_tool_executor(spec, false, native_tool_executor).await;
    if requires_native_tool_executor
        && report
            .blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("native tool executor"))
    {
        return Err(
            "spec benchmark scenario requires a native tool executor that handles the requested native tool; move this claw.migrate/claw_migrate run to daemon composition root".to_owned(),
        );
    }
    let blocked = report.operation_kind == "blocked" || report.blocked_reason.is_some();

    let mut sample = ScenarioRunSample {
        passed: true,
        blocked,
        ..ScenarioRunSample::default()
    };

    if blocked {
        if !scenario.allow_blocked {
            sample.passed = false;
        }
        if let Some(reason) = report.blocked_reason.as_deref() {
            let code =
                parse_programmatic_error_code(reason).unwrap_or_else(|| "blocked".to_owned());
            increment_error_code(&mut sample.error_codes, &code);
        } else {
            increment_error_code(&mut sample.error_codes, "blocked");
        }
        return Ok(sample);
    }

    if report.operation_kind != scenario.expected_operation_kind {
        sample.passed = false;
        increment_error_code(&mut sample.error_codes, "unexpected_operation_kind");
    }

    sample.connector_calls = report
        .outcome
        .get("connector_calls")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    sample.schema_fingerprint = extract_programmatic_schema_fingerprint(&report.outcome);

    collect_spec_step_metrics(&report.outcome, &mut sample);

    Ok(sample)
}

async fn run_circuit_half_open_pressure_once(
    connector_name: &str,
    policy: &ProgrammaticCircuitBreakerPolicy,
    failures_before_open: usize,
    recovery_successes: usize,
    poll_interval_ms: Option<u64>,
    cooldown_poll_timeout_ms: Option<u64>,
) -> CliResult<ScenarioRunSample> {
    let policies = BTreeMap::from([(connector_name.to_owned(), policy.clone())]);
    let state = Arc::new(tokio::sync::Mutex::new(BTreeMap::<
        String,
        ProgrammaticCircuitRuntimeState,
    >::new()));
    let step_id = "pressure-circuit-half-open";

    let mut sample = ScenarioRunSample {
        passed: true,
        ..ScenarioRunSample::default()
    };

    for _ in 0..failures_before_open {
        acquire_programmatic_circuit_slot(
            connector_name,
            &policies,
            &state,
            step_id,
            Some("preopen"),
        )
        .await?;
        let _ = record_programmatic_circuit_outcome(connector_name, false, &policies, &state).await;
        sample.connector_calls = sample.connector_calls.saturating_add(1);
    }

    let blocked_error = acquire_programmatic_circuit_slot(
        connector_name,
        &policies,
        &state,
        step_id,
        Some("open-check"),
    )
    .await
    .err();
    if blocked_error.is_none() {
        sample.passed = false;
        increment_error_code(&mut sample.error_codes, "circuit_not_open_after_failures");
        return Ok(sample);
    }
    increment_error_code(&mut sample.error_codes, "circuit_open");

    let poll_interval =
        Duration::from_millis(poll_interval_ms.unwrap_or(DEFAULT_CIRCUIT_POLL_INTERVAL_MS));
    let poll_timeout = Duration::from_millis(
        cooldown_poll_timeout_ms.unwrap_or(
            policy
                .cooldown_ms
                .saturating_add(DEFAULT_CIRCUIT_RECOVERY_BUFFER_MS),
        ),
    );

    let transition_started = TokioInstant::now();
    let mut observed_half_open = false;
    let mut closed_after_recovery = false;
    let mut remaining_recovery_successes = recovery_successes.max(1);

    loop {
        match acquire_programmatic_circuit_slot(
            connector_name,
            &policies,
            &state,
            step_id,
            Some("half-open-probe"),
        )
        .await
        {
            Ok(phase) => {
                if phase == "half_open" {
                    observed_half_open = true;
                    sample.connector_calls = sample.connector_calls.saturating_add(1);
                    let after = record_programmatic_circuit_outcome(
                        connector_name,
                        true,
                        &policies,
                        &state,
                    )
                    .await;
                    if after == "closed" {
                        closed_after_recovery = true;
                    }
                    remaining_recovery_successes = remaining_recovery_successes.saturating_sub(1);
                    break;
                }
                if phase == "closed" {
                    closed_after_recovery = true;
                    break;
                }
            }
            Err(_) => {
                if transition_started.elapsed() >= poll_timeout {
                    break;
                }
                sleep(poll_interval).await;
            }
        }
    }

    sample.half_open_transition_ms = Some(transition_started.elapsed().as_secs_f64() * 1_000.0);

    if !observed_half_open {
        sample.passed = false;
        increment_error_code(&mut sample.error_codes, "half_open_not_observed");
        sample.closed_after_recovery = Some(false);
        return Ok(sample);
    }

    while !closed_after_recovery && remaining_recovery_successes > 0 {
        let phase = acquire_programmatic_circuit_slot(
            connector_name,
            &policies,
            &state,
            step_id,
            Some("recovery-success"),
        )
        .await?;
        sample.connector_calls = sample.connector_calls.saturating_add(1);
        if phase == "closed" {
            closed_after_recovery = true;
            break;
        }
        let after =
            record_programmatic_circuit_outcome(connector_name, true, &policies, &state).await;
        if after == "closed" {
            closed_after_recovery = true;
            break;
        }
        remaining_recovery_successes = remaining_recovery_successes.saturating_sub(1);
    }

    sample.closed_after_recovery = Some(closed_after_recovery);
    if !closed_after_recovery {
        sample.passed = false;
        increment_error_code(&mut sample.error_codes, "circuit_not_closed_after_recovery");
    }

    Ok(sample)
}

fn summarize_programmatic_pressure_scenario(
    scenario: &ProgrammaticPressureScenario,
    iterations: usize,
    warmup_iterations: usize,
    samples: Vec<ScenarioRunSample>,
    thresholds: Option<&ProgrammaticPressureScenarioThresholds>,
    enforce_gate: bool,
) -> ProgrammaticPressureScenarioReport {
    let success_runs = samples.iter().filter(|sample| sample.passed).count();
    let failed_runs = iterations.saturating_sub(success_runs);
    let blocked_runs = samples.iter().filter(|sample| sample.blocked).count();

    let latencies: Vec<f64> = samples.iter().map(|sample| sample.latency_ms).collect();
    let latency_ms = compute_numeric_stats(&latencies);

    let total_latency_ms: f64 = latencies.iter().sum();
    let connector_calls_total: usize = samples.iter().map(|sample| sample.connector_calls).sum();
    let throughput_rps = if total_latency_ms > 0.0 {
        connector_calls_total as f64 / (total_latency_ms / 1_000.0)
    } else {
        0.0
    };

    let mut error_codes = BTreeMap::new();
    for sample in &samples {
        for (code, count) in &sample.error_codes {
            *error_codes.entry(code.clone()).or_insert(0) += count;
        }
    }

    let circuit_open_errors = error_codes.get("circuit_open").copied().unwrap_or(0);
    let circuit_open_error_ratio = if connector_calls_total > 0 {
        circuit_open_errors as f64 / connector_calls_total as f64
    } else {
        0.0
    };

    let scheduler = build_scheduler_aggregate(&samples);
    let circuit = build_circuit_aggregate(&samples);
    let schema_fingerprint = aggregate_schema_fingerprint(&samples);

    let mut report = ProgrammaticPressureScenarioReport {
        name: scenario.name.clone(),
        description: scenario.description.clone(),
        scenario_kind: scenario_kind_label(&scenario.kind).to_owned(),
        iterations,
        warmup_iterations,
        success_runs,
        failed_runs,
        blocked_runs,
        error_rate: failed_runs as f64 / iterations as f64,
        blocked_rate: blocked_runs as f64 / iterations as f64,
        connector_calls_total,
        throughput_rps,
        latency_ms,
        circuit_open_error_ratio,
        schema_fingerprint,
        scheduler,
        circuit,
        error_codes,
        gate: ProgrammaticPressureScenarioGate {
            passed: true,
            checks: Vec::new(),
            warnings: Vec::new(),
        },
    };

    report.gate = evaluate_scenario_gate(&report, thresholds, enforce_gate);
    report
}

fn build_scheduler_aggregate(
    samples: &[ScenarioRunSample],
) -> Option<ProgrammaticSchedulerAggregate> {
    let scheduler_samples: Vec<&SchedulerSnapshot> = samples
        .iter()
        .filter_map(|sample| sample.scheduler.as_ref())
        .collect();
    if scheduler_samples.is_empty() {
        return None;
    }

    let observed_runs = scheduler_samples.len();
    let peak_in_flight_max = scheduler_samples
        .iter()
        .map(|sample| sample.peak_in_flight)
        .max()
        .unwrap_or(0);
    let peak_in_flight_avg = scheduler_samples
        .iter()
        .map(|sample| sample.peak_in_flight as f64)
        .sum::<f64>()
        / observed_runs as f64;
    let budget_reductions_total = scheduler_samples
        .iter()
        .map(|sample| sample.budget_reductions)
        .sum();
    let budget_increases_total = scheduler_samples
        .iter()
        .map(|sample| sample.budget_increases)
        .sum();
    let wait_cycles_total = scheduler_samples
        .iter()
        .map(|sample| sample.wait_cycles)
        .sum();
    let min_final_in_flight_budget = scheduler_samples
        .iter()
        .map(|sample| sample.final_in_flight_budget)
        .min()
        .unwrap_or(0);
    let max_final_in_flight_budget = scheduler_samples
        .iter()
        .map(|sample| sample.final_in_flight_budget)
        .max()
        .unwrap_or(0);

    Some(ProgrammaticSchedulerAggregate {
        observed_runs,
        peak_in_flight_max,
        peak_in_flight_avg,
        budget_reductions_total,
        budget_increases_total,
        wait_cycles_total,
        min_final_in_flight_budget,
        max_final_in_flight_budget,
    })
}

fn build_circuit_aggregate(samples: &[ScenarioRunSample]) -> Option<ProgrammaticCircuitAggregate> {
    let transition_ms: Vec<f64> = samples
        .iter()
        .filter_map(|sample| sample.half_open_transition_ms)
        .collect();
    if transition_ms.is_empty() {
        return None;
    }

    let closed_samples: Vec<bool> = samples
        .iter()
        .filter_map(|sample| sample.closed_after_recovery)
        .collect();
    let closed_after_recovery_rate = if closed_samples.is_empty() {
        0.0
    } else {
        let closed = closed_samples.iter().filter(|&&value| value).count();
        closed as f64 / closed_samples.len() as f64
    };

    Some(ProgrammaticCircuitAggregate {
        observed_runs: transition_ms.len(),
        half_open_transition_ms: compute_numeric_stats(&transition_ms),
        closed_after_recovery_rate,
    })
}

fn collect_spec_step_metrics(outcome: &Value, sample: &mut ScenarioRunSample) {
    let Some(step_outputs) = outcome.get("step_outputs").and_then(Value::as_object) else {
        return;
    };

    let mut scheduler_snapshot: Option<SchedulerSnapshot> = None;

    for output in step_outputs.values() {
        if let Some(calls) = output.get("calls").and_then(Value::as_array) {
            for call in calls {
                let status = call
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if status == "error" {
                    let code = call
                        .get("error_code")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown_call_error");
                    increment_error_code(&mut sample.error_codes, code);
                }
            }
        }

        let Some(raw_scheduler) = output.get("scheduler").and_then(Value::as_object) else {
            continue;
        };

        let observed = SchedulerSnapshot {
            peak_in_flight: raw_scheduler
                .get("peak_in_flight")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            final_in_flight_budget: raw_scheduler
                .get("final_in_flight_budget")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            budget_reductions: raw_scheduler
                .get("budget_reductions")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            budget_increases: raw_scheduler
                .get("budget_increases")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
            wait_cycles: raw_scheduler
                .get("scheduler_wait_cycles")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
        };

        scheduler_snapshot = Some(match scheduler_snapshot {
            None => observed,
            Some(current) => SchedulerSnapshot {
                peak_in_flight: current.peak_in_flight.max(observed.peak_in_flight),
                final_in_flight_budget: current
                    .final_in_flight_budget
                    .min(observed.final_in_flight_budget),
                budget_reductions: current.budget_reductions + observed.budget_reductions,
                budget_increases: current.budget_increases + observed.budget_increases,
                wait_cycles: current.wait_cycles + observed.wait_cycles,
            },
        });
    }

    sample.scheduler = scheduler_snapshot;
}

fn evaluate_scenario_gate(
    report: &ProgrammaticPressureScenarioReport,
    thresholds: Option<&ProgrammaticPressureScenarioThresholds>,
    enforce_gate: bool,
) -> ProgrammaticPressureScenarioGate {
    let mut checks = Vec::new();
    let mut warnings = Vec::new();

    let Some(thresholds) = thresholds else {
        if enforce_gate {
            checks.push(ProgrammaticPressureGateCheck {
                metric: "baseline_presence".to_owned(),
                comparator: "required".to_owned(),
                threshold: 1.0,
                observed: 0.0,
                passed: false,
                baseline_threshold: None,
                detail: None,
            });
        } else {
            warnings.push("baseline thresholds missing for this scenario".to_owned());
        }
        return ProgrammaticPressureScenarioGate {
            passed: checks.iter().all(|check| check.passed),
            checks,
            warnings,
        };
    };
    let tolerance = normalized_gate_tolerance(&thresholds.tolerance);

    if let Some(threshold) = thresholds.max_error_rate {
        push_max_gate_check(
            &mut checks,
            "error_rate",
            report.error_rate,
            threshold,
            tolerance.max_ratio,
            0.0,
        );
    }
    if let Some(threshold) = thresholds.max_p95_latency_ms {
        push_optional_max_gate_check(
            &mut checks,
            &mut warnings,
            "latency_ms.p95",
            report.latency_ms.p95,
            threshold,
            tolerance.max_ratio,
            tolerance.latency_ms,
            enforce_gate,
        );
    }
    if let Some(threshold) = thresholds.max_p99_latency_ms {
        push_optional_max_gate_check(
            &mut checks,
            &mut warnings,
            "latency_ms.p99",
            report.latency_ms.p99,
            threshold,
            tolerance.max_ratio,
            tolerance.latency_ms,
            enforce_gate,
        );
    }
    if let Some(threshold) = thresholds.min_throughput_rps {
        push_min_gate_check(
            &mut checks,
            "throughput_rps",
            report.throughput_rps,
            threshold,
            tolerance.min_ratio,
        );
    }
    if let Some(threshold) = thresholds.min_peak_in_flight {
        let observed = report
            .scheduler
            .as_ref()
            .map(|value| value.peak_in_flight_max as f64);
        push_optional_min_gate_check(
            &mut checks,
            &mut warnings,
            "scheduler.peak_in_flight_max",
            observed,
            threshold,
            tolerance.min_ratio,
            enforce_gate,
        );
    }
    if let Some(threshold) = thresholds.max_circuit_open_error_ratio {
        push_max_gate_check(
            &mut checks,
            "circuit_open_error_ratio",
            report.circuit_open_error_ratio,
            threshold,
            tolerance.max_ratio,
            0.0,
        );
    }
    if let Some(threshold) = thresholds.max_half_open_p95_ms {
        let observed = report
            .circuit
            .as_ref()
            .and_then(|value| value.half_open_transition_ms.p95);
        push_optional_max_gate_check(
            &mut checks,
            &mut warnings,
            "circuit.half_open_transition_ms.p95",
            observed,
            threshold,
            tolerance.max_ratio,
            tolerance.latency_ms,
            enforce_gate,
        );
    }
    if report.scenario_kind == "spec_run" {
        if let Some(expected) = thresholds.expected_schema_fingerprint.as_deref() {
            push_schema_fingerprint_gate_check(
                &mut checks,
                &mut warnings,
                report.schema_fingerprint.as_deref(),
                expected,
            );
        } else if enforce_gate {
            checks.push(ProgrammaticPressureGateCheck {
                metric: "schema_fingerprint_baseline_presence".to_owned(),
                comparator: "required".to_owned(),
                threshold: 1.0,
                observed: 0.0,
                passed: false,
                baseline_threshold: None,
                detail: Some(
                    "expected_schema_fingerprint missing for spec_run scenario in baseline"
                        .to_owned(),
                ),
            });
        } else {
            warnings.push(
                "expected_schema_fingerprint missing for spec_run scenario in baseline".to_owned(),
            );
        }
    }

    ProgrammaticPressureScenarioGate {
        passed: checks.iter().all(|check| check.passed),
        checks,
        warnings,
    }
}

fn push_max_gate_check(
    checks: &mut Vec<ProgrammaticPressureGateCheck>,
    metric: &str,
    observed: f64,
    threshold: f64,
    tolerance_ratio: f64,
    additive_tolerance: f64,
) {
    let effective_threshold = threshold * (1.0 + tolerance_ratio) + additive_tolerance;
    checks.push(ProgrammaticPressureGateCheck {
        metric: metric.to_owned(),
        comparator: "<=".to_owned(),
        threshold: effective_threshold,
        observed,
        passed: observed <= effective_threshold,
        baseline_threshold: Some(threshold),
        detail: None,
    });
}

fn push_min_gate_check(
    checks: &mut Vec<ProgrammaticPressureGateCheck>,
    metric: &str,
    observed: f64,
    threshold: f64,
    tolerance_ratio: f64,
) {
    let effective_threshold = threshold * (1.0 - tolerance_ratio);
    checks.push(ProgrammaticPressureGateCheck {
        metric: metric.to_owned(),
        comparator: ">=".to_owned(),
        threshold: effective_threshold,
        observed,
        passed: observed >= effective_threshold,
        baseline_threshold: Some(threshold),
        detail: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_optional_max_gate_check(
    checks: &mut Vec<ProgrammaticPressureGateCheck>,
    warnings: &mut Vec<String>,
    metric: &str,
    observed: Option<f64>,
    threshold: f64,
    tolerance_ratio: f64,
    additive_tolerance: f64,
    enforce_gate: bool,
) {
    match observed {
        Some(value) => push_max_gate_check(
            checks,
            metric,
            value,
            threshold,
            tolerance_ratio,
            additive_tolerance,
        ),
        None => {
            if enforce_gate {
                checks.push(ProgrammaticPressureGateCheck {
                    metric: metric.to_owned(),
                    comparator: "<=".to_owned(),
                    threshold,
                    observed: f64::INFINITY,
                    passed: false,
                    baseline_threshold: Some(threshold),
                    detail: None,
                });
            } else {
                warnings.push(format!("metric {metric} not observed in report"));
            }
        }
    }
}

fn push_optional_min_gate_check(
    checks: &mut Vec<ProgrammaticPressureGateCheck>,
    warnings: &mut Vec<String>,
    metric: &str,
    observed: Option<f64>,
    threshold: f64,
    tolerance_ratio: f64,
    enforce_gate: bool,
) {
    match observed {
        Some(value) => push_min_gate_check(checks, metric, value, threshold, tolerance_ratio),
        None => {
            if enforce_gate {
                checks.push(ProgrammaticPressureGateCheck {
                    metric: metric.to_owned(),
                    comparator: ">=".to_owned(),
                    threshold,
                    observed: 0.0,
                    passed: false,
                    baseline_threshold: Some(threshold),
                    detail: None,
                });
            } else {
                warnings.push(format!("metric {metric} not observed in report"));
            }
        }
    }
}

fn normalized_gate_tolerance(
    tolerance: &ProgrammaticPressureGateTolerance,
) -> ProgrammaticPressureGateTolerance {
    ProgrammaticPressureGateTolerance {
        max_ratio: normalize_ratio_tolerance(tolerance.max_ratio),
        min_ratio: normalize_ratio_tolerance(tolerance.min_ratio),
        latency_ms: normalize_non_negative_tolerance(tolerance.latency_ms),
    }
}

fn push_schema_fingerprint_gate_check(
    checks: &mut Vec<ProgrammaticPressureGateCheck>,
    warnings: &mut Vec<String>,
    observed: Option<&str>,
    expected: &str,
) {
    match observed {
        Some(actual) => {
            let passed = actual == expected;
            checks.push(ProgrammaticPressureGateCheck {
                metric: "schema_fingerprint".to_owned(),
                comparator: "==".to_owned(),
                threshold: 1.0,
                observed: if passed { 1.0 } else { 0.0 },
                passed,
                baseline_threshold: Some(1.0),
                detail: Some(format!("expected={expected}, observed={actual}")),
            });
        }
        None => {
            checks.push(ProgrammaticPressureGateCheck {
                metric: "schema_fingerprint".to_owned(),
                comparator: "==".to_owned(),
                threshold: 1.0,
                observed: 0.0,
                passed: false,
                baseline_threshold: Some(1.0),
                detail: Some(format!("expected={expected}, observed=<missing>")),
            });
            warnings.push("schema fingerprint is unavailable for this scenario report".to_owned());
        }
    }
}

fn aggregate_schema_fingerprint(samples: &[ScenarioRunSample]) -> Option<String> {
    let mut unique = samples
        .iter()
        .filter_map(|sample| sample.schema_fingerprint.clone())
        .collect::<Vec<_>>();
    unique.sort();
    unique.dedup();
    match unique.len() {
        0 => None,
        1 => unique.into_iter().next(),
        _ => {
            let joined = unique.join("|");
            Some(format!("multi:{}", sha256_hex(&joined)))
        }
    }
}

fn extract_programmatic_schema_fingerprint(outcome: &Value) -> Option<String> {
    let step_outputs = outcome.get("step_outputs")?;
    let descriptor = schema_descriptor(step_outputs);
    let encoded = serde_json::to_string(&descriptor).ok()?;
    Some(sha256_hex(&encoded))
}

fn schema_descriptor(value: &Value) -> Value {
    match value {
        Value::Null => Value::String("null".to_owned()),
        Value::Bool(_) => Value::String("bool".to_owned()),
        Value::Number(_) => Value::String("number".to_owned()),
        Value::String(_) => Value::String("string".to_owned()),
        Value::Array(items) => {
            let mut normalized = items
                .iter()
                .map(schema_descriptor)
                .map(|schema| {
                    let key = serde_json::to_string(&schema).unwrap_or_else(|_| "null".to_owned());
                    (key, schema)
                })
                .collect::<Vec<_>>();
            normalized.sort_by(|left, right| left.0.cmp(&right.0));
            normalized.dedup_by(|left, right| left.0 == right.0);
            Value::Object(serde_json::Map::from_iter([(
                "array".to_owned(),
                Value::Array(normalized.into_iter().map(|(_, schema)| schema).collect()),
            )]))
        }
        Value::Object(entries) => {
            let ordered = entries
                .iter()
                .map(|(key, entry)| (key.clone(), schema_descriptor(entry)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(serde_json::Map::from_iter(ordered))
        }
    }
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

fn normalize_ratio_tolerance(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

fn normalize_non_negative_tolerance(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.max(0.0)
}

fn scenario_kind_label(kind: &ProgrammaticPressureScenarioKind) -> &'static str {
    match kind {
        ProgrammaticPressureScenarioKind::SpecRun { .. } => "spec_run",
        ProgrammaticPressureScenarioKind::CircuitHalfOpen { .. } => "circuit_half_open",
    }
}

fn parse_programmatic_error_code(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let marker = "programmatic_error[";
    let start = lower.find(marker)? + marker.len();
    let rest = &lower[start..];
    let end = rest.find(']')?;
    Some(rest[..end].to_owned())
}

fn increment_error_code(map: &mut BTreeMap<String, usize>, code: &str) {
    *map.entry(code.to_owned()).or_insert(0) += 1;
}

fn compute_numeric_stats(values: &[f64]) -> NumericStats {
    if values.is_empty() {
        return NumericStats::default();
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));

    let sum: f64 = sorted.iter().sum();
    NumericStats {
        count: sorted.len(),
        min: sorted.first().copied(),
        max: sorted.last().copied(),
        avg: Some(sum / sorted.len() as f64),
        p50: Some(percentile(&sorted, 0.50)),
        p95: Some(percentile(&sorted, 0.95)),
        p99: Some(percentile(&sorted, 0.99)),
    }
}

fn percentile(sorted_values: &[f64], ratio: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values.first().copied().unwrap_or(0.0);
    }

    let clamped = ratio.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len().saturating_sub(1) as f64);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let lower_val = sorted_values.get(lower).copied().unwrap_or(0.0);
    let upper_val = sorted_values.get(upper).copied().unwrap_or(lower_val);
    if lower == upper {
        return lower_val;
    }

    let weight = rank - lower as f64;
    lower_val + (upper_val - lower_val) * weight
}

fn read_json_file<T: DeserializeOwned>(path: &str) -> CliResult<T> {
    let raw =
        fs::read_to_string(path).map_err(|error| format!("failed to read {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse {path}: {error}"))
}

fn write_json_file<T: Serialize>(path: &str, value: &T) -> CliResult<()> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize JSON value for output file failed: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory failed: {error}"))?;
    }
    fs::write(path, serialized)
        .map_err(|error| format!("write JSON output file failed: {error}"))?;
    Ok(())
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

#[cfg(test)]
fn next_benchmark_temp_suffix() -> u64 {
    static BENCHMARK_TEMP_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    BENCHMARK_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
fn benchmark_temp_root(prefix: &str, parent: Option<&Path>) -> PathBuf {
    let parent = match parent {
        Some(parent) => parent.to_path_buf(),
        None => std::env::temp_dir(),
    };
    parent.join(format!(
        "{prefix}-{}-{}-{}",
        current_epoch_seconds(),
        std::process::id(),
        next_benchmark_temp_suffix()
    ))
}

fn resolve_memory_context_benchmark_temp_root(
    output_path: &str,
    temp_root: Option<&str>,
) -> CliResult<ResolvedMemoryContextBenchmarkTempRoot> {
    let current_exe = std::env::current_exe().ok();
    resolve_memory_context_benchmark_temp_root_with_exe(
        output_path,
        temp_root,
        current_exe.as_deref(),
    )
}

fn resolve_memory_context_benchmark_temp_root_with_exe(
    output_path: &str,
    temp_root: Option<&str>,
    current_exe: Option<&Path>,
) -> CliResult<ResolvedMemoryContextBenchmarkTempRoot> {
    if let Some(temp_root) = temp_root {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: PathBuf::from(temp_root),
            source: MemoryContextBenchmarkTempRootSource::Explicit,
        });
    }

    if let Some(current_exe) = current_exe
        && let Some(profile_dir) = current_exe.parent()
        && matches!(
            profile_dir.file_name().and_then(|name| name.to_str()),
            Some("debug" | "release")
        )
        && let Some(target_dir) = profile_dir.parent()
    {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: target_dir.join("tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
        });
    }

    let output_path = Path::new(output_path);
    let starts_in_target_dir = matches!(
        output_path.components().next(),
        Some(std::path::Component::Normal(component)) if component == "target"
    );
    if starts_in_target_dir
        && let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        return Ok(ResolvedMemoryContextBenchmarkTempRoot {
            path: parent.join("tmp-local"),
            source: MemoryContextBenchmarkTempRootSource::OutputParent,
        });
    }

    Ok(ResolvedMemoryContextBenchmarkTempRoot {
        path: std::env::temp_dir(),
        source: MemoryContextBenchmarkTempRootSource::SystemTemp,
    })
}

fn default_pressure_iterations() -> usize {
    DEFAULT_PRESSURE_ITERATIONS
}

fn default_pressure_warmup_iterations() -> usize {
    DEFAULT_PRESSURE_WARMUP_ITERATIONS
}

fn default_pressure_expected_operation_kind() -> String {
    "programmatic_tool_call".to_owned()
}

fn default_failures_before_open() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn percentile_interpolates_expected_points() {
        let values = vec![10.0, 20.0, 30.0, 40.0];
        assert_eq!(percentile(&values, 0.0), 10.0);
        assert_eq!(percentile(&values, 1.0), 40.0);
        assert!((percentile(&values, 0.50) - 25.0).abs() < f64::EPSILON);
        assert!((percentile(&values, 0.95) - 38.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_programmatic_error_code_extracts_typed_code() {
        let raw = "programmatic_error[circuit_open]: connector webhook blocked";
        assert_eq!(
            parse_programmatic_error_code(raw),
            Some("circuit_open".to_owned())
        );
    }

    #[test]
    fn parse_ps_rss_kib_output_extracts_first_non_empty_numeric_value() {
        assert_eq!(parse_ps_rss_kib_output("  12345\n"), Some(12_345.0));
        assert_eq!(parse_ps_rss_kib_output("\n  6789 extra\n"), Some(6_789.0));
    }

    #[test]
    fn parse_ps_rss_kib_output_rejects_blank_or_invalid_values() {
        assert_eq!(parse_ps_rss_kib_output(""), None);
        assert_eq!(parse_ps_rss_kib_output("  \n"), None);
        assert_eq!(parse_ps_rss_kib_output("rss\n"), None);
    }

    #[test]
    fn compute_rss_step_delta_kib_clamps_negative_and_propagates_missing_samples() {
        assert_eq!(
            compute_rss_step_delta_kib(Some(100.0), Some(112.0)),
            Some(12.0)
        );
        assert_eq!(
            compute_rss_step_delta_kib(Some(112.0), Some(100.0)),
            Some(0.0)
        );
        assert_eq!(compute_rss_step_delta_kib(None, Some(100.0)), None);
        assert_eq!(compute_rss_step_delta_kib(Some(100.0), None), None);
    }

    #[test]
    fn format_optional_decimal_returns_na_when_value_missing() {
        assert_eq!(format_optional_decimal(Some(12.34), 1), "12.3");
        assert_eq!(format_optional_decimal(None, 1), "n/a");
    }

    #[test]
    fn memory_context_soft_warnings_ignore_cover_path_noise_floor() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn memory_context_soft_warnings_flag_cover_path_regression_beyond_noise_floor() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.24),
            Some(0.083),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("summary_window_cover"));
        assert!(warnings[0].contains("soft thresholds"));
        assert!(!warnings[0].contains("suite-noisy"));
    }

    #[test]
    fn memory_context_soft_warnings_ignore_marginal_cover_regression_when_suite_is_noisy() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.177),
            Some(0.150),
            16,
            true,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            Some(1.496),
            Some(1.897),
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.contains("summary_window_cover")),
            "expected marginal cover-path regressions to stay silent when the surrounding suite is already too noisy for path-specific attribution"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("speedup_ratio_p95")),
            "expected suite-noise warnings to remain visible when cover-path specificity is suppressed"
        );
    }

    #[test]
    fn memory_context_soft_warnings_keep_large_cover_regression_even_when_suite_is_noisy() {
        let warnings = build_memory_context_soft_warnings(
            Some(3.012),
            Some(2.074),
            16,
            true,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            Some(2.670),
            Some(2.773),
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("summary_window_cover")),
            "expected clearly excessive cover-path regressions to keep their dedicated warning even on a noisy host"
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("summary_window_cover")
                    && warning.contains("suite-noisy")),
            "expected non-marginal cover regressions on noisy suites to stay visible but be qualified as suite-noisy"
        );
    }

    #[test]
    fn memory_context_soft_warnings_flag_budget_change_path_regression() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(1.12),
            Some(1.12),
            16,
            Some(0.91),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("summary_rebuild_budget_change"));
        assert!(warnings[0].contains("full rebuild"));
    }

    #[test]
    fn memory_context_soft_warnings_ignore_budget_change_when_workload_adjusted_ratio_is_stable() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(1.12),
            Some(0.58),
            16,
            Some(0.91),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert!(
            warnings
                .iter()
                .all(|warning| !warning.contains("summary_rebuild_budget_change")),
            "expected budget-change warnings to stay quiet when a larger rebuilt summary fully explains the raw latency ratio"
        );
    }

    #[test]
    fn memory_context_soft_warnings_flag_metadata_realign_regression() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(1.18),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("summary_metadata_realign"));
        assert!(warnings[0].contains("budget-change rebuild"));
        assert!(!warnings[0].contains("suite-noisy"));
    }

    #[test]
    fn memory_context_soft_warnings_require_stable_sample_size() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.24),
            Some(0.083),
            4,
            false,
            Some(1.12),
            Some(1.12),
            4,
            Some(1.18),
            4,
            None,
            None,
            None,
            None,
            None,
            None,
            2,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn memory_context_soft_warnings_flag_system_temp_root_fallback() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            None,
            None,
            None,
            None,
            None,
            5,
            1.10,
            None,
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::SystemTemp,
            Path::new("/tmp"),
        );

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("benchmark_temp_root"));
        assert!(warnings[0].contains("system temp"));
    }

    #[test]
    fn memory_context_soft_warnings_keep_speedup_warning_generic_without_rebuild_noise() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            Some(0.84),
            Some(0.19),
            None,
            None,
            None,
            5,
            1.10,
            Some(&MemoryContextColdPathNoiseAttribution {
                phase: "copy_db_ms".to_owned(),
                range_over_p50: 0.19,
            }),
            None,
            None,
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("speedup_ratio_p95"));
        assert!(!warnings[0].contains("dominant summary_rebuild cold-path noise"));
    }

    #[test]
    fn memory_context_soft_warnings_expand_target_load_noise_subphase_labels() {
        let warnings = build_memory_context_soft_warnings(
            Some(1.04),
            Some(0.012),
            16,
            false,
            Some(0.82),
            Some(0.82),
            16,
            Some(0.91),
            16,
            None,
            Some(0.84),
            Some(0.91),
            None,
            None,
            None,
            5,
            1.10,
            Some(&MemoryContextColdPathNoiseAttribution {
                phase: "target_load_ms".to_owned(),
                range_over_p50: 0.91,
            }),
            None,
            Some(&MemoryContextLoadNoiseAttribution {
                phase: "summary_catch_up_ms".to_owned(),
                range_over_p50: 0.88,
            }),
            None,
            None,
            None,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir,
            Path::new("target/codex-memory-bench-red/tmp-local"),
        );

        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|warning| {
            warning.contains("summary_rebuild suite p95")
                && warning.contains("target_load_ms/summary_catch_up_ms")
        }));
    }

    #[test]
    fn memory_context_benchmark_report_tracks_append_window_only_baselines() {
        let shape = MemoryContextShape {
            entry_count: 7,
            turn_entries: 6,
            summary_chars: 256,
            payload_chars: 768,
        };
        let suite_runs = vec![
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 1024,
                window_only_samples: vec![1.0, 1.2],
                summary_window_cover_samples: vec![1.05, 1.25],
                summary_rebuild_samples: vec![2.0, 2.2],
                summary_rebuild_budget_change_samples: vec![1.3, 1.4],
                summary_metadata_realign_samples: vec![1.2, 1.25],
                summary_steady_state_samples: vec![0.7, 0.75],
                window_shrink_catch_up_samples: vec![0.9, 0.95],
                window_only_append_pre_overflow_samples: vec![0.8, 0.82],
                window_only_append_cold_overflow_samples: vec![0.85, 0.9],
                summary_append_pre_overflow_samples: vec![1.1, 1.15],
                summary_append_cold_overflow_samples: vec![1.4, 1.5],
                summary_append_saturated_samples: vec![1.0, 1.05],
                window_only_rss_deltas_kib: vec![0.0, 16.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 16.0],
                summary_rebuild_rss_deltas_kib: vec![32.0, 48.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![16.0, 32.0],
                summary_metadata_realign_rss_deltas_kib: vec![16.0, 16.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![16.0, 16.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![16.0, 16.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![16.0, 32.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![16.0, 32.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![32.0, 32.0],
                summary_append_saturated_rss_deltas_kib: vec![16.0, 16.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 1024,
                window_only_samples: vec![1.1, 1.3],
                summary_window_cover_samples: vec![1.15, 1.35],
                summary_rebuild_samples: vec![2.1, 2.4],
                summary_rebuild_budget_change_samples: vec![1.35, 1.45],
                summary_metadata_realign_samples: vec![1.22, 1.28],
                summary_steady_state_samples: vec![0.72, 0.77],
                window_shrink_catch_up_samples: vec![0.92, 1.0],
                window_only_append_pre_overflow_samples: vec![0.82, 0.86],
                window_only_append_cold_overflow_samples: vec![0.9, 0.94],
                summary_append_pre_overflow_samples: vec![1.18, 1.22],
                summary_append_cold_overflow_samples: vec![1.48, 1.58],
                summary_append_saturated_samples: vec![1.02, 1.08],
                window_only_rss_deltas_kib: vec![0.0, 16.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 16.0],
                summary_rebuild_rss_deltas_kib: vec![32.0, 48.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![16.0, 32.0],
                summary_metadata_realign_rss_deltas_kib: vec![16.0, 16.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![16.0, 16.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![16.0, 16.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![16.0, 32.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![16.0, 32.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![32.0, 48.0],
                summary_append_saturated_rss_deltas_kib: vec![16.0, 16.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            2,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert!(
            report_json
                .get("window_only_append_pre_overflow_latency_ms")
                .is_some()
        );
        assert!(
            report_json
                .get("window_only_append_cold_overflow_latency_ms")
                .is_some()
        );
        assert!(
            report_json
                .get("window_only_append_pre_overflow_rss_delta_kib")
                .is_some()
        );
        assert!(
            report_json
                .get("window_only_append_cold_overflow_rss_delta_kib")
                .is_some()
        );
        assert!(
            report_json
                .get("flattened_sample_ratios")
                .and_then(|value| {
                    value.get(
                        "summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95",
                    )
                })
                .is_some()
        );
        assert!(
            report_json
                .get("aggregated_ratios")
                .and_then(|value| {
                    value.get(
                        "summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95",
                    )
                })
                .is_some()
        );
        assert!(
            report_json
                .get("flattened_sample_ratios")
                .and_then(|value| {
                    value.get("summary_append_pre_overflow_vs_window_only_ratio_p95")
                })
                .is_some()
        );
        assert!(
            report_json
                .get("flattened_sample_ratios")
                .and_then(|value| {
                    value.get("summary_append_cold_overflow_vs_window_only_ratio_p95")
                })
                .is_some()
        );
        assert!(
            report_json
                .get("aggregated_p95_median_ms")
                .and_then(|value| value.get("window_only_append_pre_overflow"))
                .is_some()
        );
        assert!(
            report_json
                .get("aggregated_p95_median_ms")
                .and_then(|value| value.get("window_only_append_cold_overflow"))
                .is_some()
        );
        assert!(
            report_json
                .get("aggregated_ratios")
                .and_then(|value| {
                    value.get("summary_append_pre_overflow_vs_window_only_ratio_p95")
                })
                .is_some()
        );
        assert!(
            report_json
                .get("aggregated_ratios")
                .and_then(|value| {
                    value.get("summary_append_cold_overflow_vs_window_only_ratio_p95")
                })
                .is_some()
        );
    }

    #[test]
    fn memory_context_benchmark_report_separates_flattened_and_aggregated_ratio_views() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let suite_runs = vec![
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![10.0, 10.0],
                summary_rebuild_budget_change_samples: vec![5.0, 5.0],
                summary_metadata_realign_samples: vec![2.0, 2.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![10.0, 10.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![11.0, 11.0],
                summary_rebuild_budget_change_samples: vec![5.5, 5.5],
                summary_metadata_realign_samples: vec![2.2, 2.2],
                summary_steady_state_samples: vec![10.0, 10.0],
                window_shrink_catch_up_samples: vec![9.0, 9.0],
                window_only_append_pre_overflow_samples: vec![10.0, 10.0],
                window_only_append_cold_overflow_samples: vec![10.0, 10.0],
                summary_append_pre_overflow_samples: vec![10.0, 10.0],
                summary_append_cold_overflow_samples: vec![11.0, 11.0],
                summary_append_saturated_samples: vec![10.0, 10.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            2,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        let flattened_append_cold_ratio = report_json
            .get("flattened_sample_ratios")
            .and_then(|value| value.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .and_then(Value::as_f64)
            .expect("flattened append-cold ratio should be present");
        let aggregated_append_cold_ratio = report_json
            .get("aggregated_ratios")
            .and_then(|value| value.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .and_then(Value::as_f64)
            .expect("aggregated append-cold ratio should be present");

        assert!(
            report_json
                .get("summary_append_cold_overflow_vs_window_only_ratio_p95")
                .is_none(),
            "expected the report root to stop exposing ambiguous ratio fields once flattened_sample_ratios is available"
        );
        assert!(
            (flattened_append_cold_ratio - aggregated_append_cold_ratio).abs() > 1.0,
            "expected the fixture to preserve a visible difference between flattened-sample and aggregated ratio views"
        );
    }

    #[test]
    fn memory_context_benchmark_report_uses_aggregated_ratio_view_for_soft_warnings() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |cover_samples: Vec<f64>,
                          budget_samples: Vec<f64>,
                          metadata_samples: Vec<f64>| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0, 1.0, 1.0],
                summary_window_cover_samples: cover_samples,
                summary_rebuild_samples: vec![4.0, 4.0, 4.0, 4.0],
                summary_rebuild_budget_change_samples: budget_samples,
                summary_metadata_realign_samples: metadata_samples,
                summary_steady_state_samples: vec![1.0, 1.0, 1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0, 2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0, 1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0, 1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0, 0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(
                vec![1.0, 1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0, 1.0],
            ),
            make_suite(
                vec![1.0, 1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0, 1.0],
            ),
            make_suite(
                vec![1.0, 1.0, 2.0, 2.0],
                vec![1.0, 1.0, 1.0, 1.0],
                vec![1.0, 1.0, 2.0, 2.0],
            ),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert_eq!(
            report
                .flattened_sample_ratios
                .summary_window_cover_vs_window_only_ratio_p95,
            Some(2.0)
        );
        assert_eq!(
            report
                .aggregated_ratios
                .summary_window_cover_vs_window_only_ratio_p95,
            Some(1.0)
        );
        assert_eq!(
            report
                .flattened_sample_ratios
                .summary_metadata_realign_vs_budget_change_ratio_p95,
            Some(2.0)
        );
        assert_eq!(
            report
                .aggregated_ratios
                .summary_metadata_realign_vs_budget_change_ratio_p95,
            Some(1.0)
        );
        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("summary_window_cover")),
            "expected cover-path warnings to key off aggregated suite-median ratios instead of flattened tails"
        );
        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("summary_metadata_realign")),
            "expected metadata-realign warnings to key off aggregated suite-median ratios instead of flattened tails"
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_suite_p95_summaries_for_noise_analysis() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let suite_runs = vec![
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.2],
                summary_window_cover_samples: vec![0.9, 1.1],
                summary_rebuild_samples: vec![2.0, 2.2],
                summary_rebuild_budget_change_samples: vec![1.0, 1.1],
                summary_metadata_realign_samples: vec![0.8, 0.9],
                summary_steady_state_samples: vec![0.5, 0.55],
                window_shrink_catch_up_samples: vec![0.7, 0.75],
                window_only_append_pre_overflow_samples: vec![0.8, 0.82],
                window_only_append_cold_overflow_samples: vec![0.9, 0.95],
                summary_append_pre_overflow_samples: vec![0.7, 0.74],
                summary_append_cold_overflow_samples: vec![1.1, 1.2],
                summary_append_saturated_samples: vec![0.6, 0.65],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![2.0, 2.2],
                summary_window_cover_samples: vec![2.1, 2.3],
                summary_rebuild_samples: vec![3.0, 3.2],
                summary_rebuild_budget_change_samples: vec![1.5, 1.7],
                summary_metadata_realign_samples: vec![1.2, 1.3],
                summary_steady_state_samples: vec![0.9, 1.0],
                window_shrink_catch_up_samples: vec![1.1, 1.2],
                window_only_append_pre_overflow_samples: vec![1.3, 1.4],
                window_only_append_cold_overflow_samples: vec![1.4, 1.5],
                summary_append_pre_overflow_samples: vec![1.0, 1.05],
                summary_append_cold_overflow_samples: vec![1.8, 1.9],
                summary_append_saturated_samples: vec![0.9, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            },
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            2,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        let suite_summaries = report_json
            .get("suite_p95_summaries")
            .and_then(Value::as_array)
            .expect("suite p95 summaries should be present");
        assert_eq!(suite_summaries.len(), 2);
        assert!(
            suite_summaries[0]
                .get("summary_append_cold_overflow")
                .and_then(Value::as_f64)
                .is_some(),
            "expected each suite summary to expose scenario-level p95s for direct noise inspection"
        );
        assert!(
            suite_summaries[0]
                .get("summary_append_cold_overflow_vs_window_only_ratio_p95")
                .and_then(Value::as_f64)
                .is_some(),
            "expected each suite summary to expose ratio-level p95s for direct noise inspection"
        );
        assert!(
            suite_summaries[0]
                .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
                .and_then(Value::as_f64)
                .is_some(),
            "expected each suite summary to expose workload-adjusted budget-change ratios for direct noise inspection"
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_suite_stability_summary() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |window_only: f64,
             summary_window_cover: f64,
             summary_rebuild: f64,
             summary_rebuild_budget_change: f64,
             summary_metadata_realign: f64,
             summary_steady_state: f64,
             window_shrink_catch_up: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![window_only, window_only],
                summary_window_cover_samples: vec![summary_window_cover, summary_window_cover],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![
                    summary_rebuild_budget_change,
                    summary_rebuild_budget_change,
                ],
                summary_metadata_realign_samples: vec![
                    summary_metadata_realign,
                    summary_metadata_realign,
                ],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![
                    window_shrink_catch_up,
                    window_shrink_catch_up,
                ],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(1.0, 0.8, 4.0, 2.0, 1.0, 1.0, 2.0),
            make_suite(2.0, 1.6, 5.0, 2.5, 1.5, 1.25, 2.5),
            make_suite(3.0, 2.4, 6.0, 3.0, 2.0, 1.5, 3.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("suite_stability")
                .and_then(|value| value.get("window_only_p95_ms"))
                .and_then(|value| value.get("count"))
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            report_json
                .get("suite_stability")
                .and_then(|value| value.get("window_only_p95_ms"))
                .and_then(|value| value.get("range"))
                .and_then(Value::as_f64),
            Some(2.0)
        );
        assert_eq!(
            report_json
                .get("suite_stability")
                .and_then(|value| value.get("summary_window_cover_vs_window_only_ratio_p95"))
                .and_then(|value| value.get("range"))
                .and_then(Value::as_f64),
            Some(0.0)
        );
        assert_eq!(
            report_json
                .get("suite_stability")
                .and_then(|value| value.get("speedup_ratio_p95"))
                .and_then(|value| value.get("max_over_p50"))
                .and_then(Value::as_f64),
            Some(1.0)
        );
    }

    #[test]
    fn memory_context_benchmark_report_warns_when_speedup_ratio_is_suite_noisy() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
            seed_db_bytes: 512,
            window_only_samples: vec![1.0, 1.0],
            summary_window_cover_samples: vec![1.0, 1.0],
            summary_rebuild_samples: vec![4.0, 4.0],
            summary_rebuild_budget_change_samples: vec![2.0, 2.0],
            summary_metadata_realign_samples: vec![1.0, 1.0],
            summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
            window_shrink_catch_up_samples: vec![2.0, 2.0],
            window_only_append_pre_overflow_samples: vec![1.0, 1.0],
            window_only_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_pre_overflow_samples: vec![1.0, 1.0],
            summary_append_cold_overflow_samples: vec![1.0, 1.0],
            summary_append_saturated_samples: vec![1.0, 1.0],
            window_only_rss_deltas_kib: vec![0.0, 0.0],
            summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
            summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
            summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
            window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
            summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
            summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples::default(
            ),
            summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
            window_only_shape: shape,
            summary_window_cover_shape: shape,
            summary_rebuild_shape: shape,
            summary_rebuild_budget_change_shape: shape,
            summary_metadata_realign_shape: shape,
            summary_steady_state_shape: shape,
            window_shrink_catch_up_shape: shape,
        };
        let suite_runs = vec![make_suite(1.0), make_suite(2.0), make_suite(4.0)];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report
                .gate
                .warnings
                .iter()
                .any(|warning| warning.contains("speedup_ratio_p95")),
            "expected suite stability warning for noisy speedup ratio"
        );
    }

    #[test]
    fn memory_context_benchmark_report_qualifies_cover_warning_under_suite_noise() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |window_only: f64, summary_window_cover: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![window_only, window_only],
                summary_window_cover_samples: vec![summary_window_cover, summary_window_cover],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(0.4300038, 0.31504375),
            make_suite(0.44856565, 1.1060792),
            make_suite(0.8337033, 0.45344445),
            make_suite(0.9723356, 1.72131875),
            make_suite(0.6513814, 1.0745410),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        let cover_warning = report
            .gate
            .warnings
            .iter()
            .find(|warning| warning.contains("summary_window_cover"))
            .expect("expected cover warning");
        assert!(
            cover_warning.contains("suite-noisy"),
            "expected noisy cover-path comparisons to be qualified as suite-noisy instead of being presented as a direct product regression"
        );
        assert!(
            !cover_warning.contains("redundant summary materialization or checkpoint work"),
            "expected suite-noisy cover warnings to avoid over-specific product-cause guidance"
        );
    }

    #[test]
    fn memory_context_benchmark_report_qualifies_metadata_realign_warning_under_suite_noise() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |summary_rebuild_budget_change: f64, summary_metadata_realign: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![
                    summary_rebuild_budget_change,
                    summary_rebuild_budget_change,
                ],
                summary_metadata_realign_samples: vec![
                    summary_metadata_realign,
                    summary_metadata_realign,
                ],
                summary_steady_state_samples: vec![2.0, 2.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(1.36408355, 0.31361875),
            make_suite(0.45087515, 3.65480625),
            make_suite(0.65107080, 1.56425160),
            make_suite(0.49981005, 0.98310625),
            make_suite(0.70441715, 0.34252745),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        let metadata_warning = report
            .gate
            .warnings
            .iter()
            .find(|warning| warning.contains("summary_metadata_realign"))
            .expect("expected metadata-realign warning");
        assert!(
            metadata_warning.contains("suite-noisy"),
            "expected noisy metadata/budget-change comparisons to be qualified as suite-noisy instead of being presented as a direct product regression"
        );
        assert!(
            !metadata_warning.contains("accidental summary body rewrites"),
            "expected suite-noisy metadata warning to avoid over-specific product-cause guidance"
        );
    }

    #[test]
    fn memory_context_benchmark_report_suppresses_speedup_warning_when_copy_noise_dominates_clear_wins()
     {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |summary_rebuild: f64,
                          summary_steady_state: f64,
                          copy_db_ms: f64,
                          target_load_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![copy_db_ms, copy_db_ms],
                    target_load_ms: vec![target_load_ms, target_load_ms],
                    target_load_summary_rebuild_ms: vec![target_load_ms, target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(4.0, 0.40, 1.0, 3.0),
            make_suite(8.0, 1.60, 5.0, 8.0),
            make_suite(12.0, 3.00, 9.0, 12.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("speedup_ratio_p95")),
            "expected non-marginal speedup wins to ignore suite-noise warnings when copy_db_ms is the dominant rebuild-noise source"
        );
        assert!(
            report
                .gate
                .warnings
                .iter()
                .any(|warning| warning.contains("summary_rebuild suite p95")),
            "expected summary_rebuild instability warning to remain visible for the underlying cold-path noise"
        );
    }

    #[test]
    fn memory_context_benchmark_report_suppresses_speedup_warning_when_bootstrap_noise_dominates_clear_wins()
     {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |summary_rebuild: f64,
                          summary_steady_state: f64,
                          target_bootstrap_ms: f64,
                          schema_upgrade_ms: f64,
                          target_load_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_bootstrap_ms: vec![target_bootstrap_ms, target_bootstrap_ms],
                    target_bootstrap_schema_upgrade_ms: vec![schema_upgrade_ms, schema_upgrade_ms],
                    target_load_ms: vec![target_load_ms, target_load_ms],
                    target_load_summary_rebuild_ms: vec![target_load_ms, target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(4.0, 0.20, 1.0, 1.0, 3.0),
            make_suite(8.0, 0.80, 8.0, 8.0, 4.0),
            make_suite(12.0, 1.20, 16.0, 16.0, 5.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("speedup_ratio_p95")),
            "expected non-marginal speedup wins to ignore suite-noise warnings when bootstrap noise dominates rebuild instability"
        );
        assert!(
            report
                .gate
                .warnings
                .iter()
                .any(|warning| warning.contains("summary_rebuild suite p95")),
            "expected summary_rebuild instability warning to remain visible when bootstrap noise dominates"
        );
    }

    #[test]
    fn memory_context_benchmark_report_suppresses_speedup_warning_for_tiny_hot_path_denominator_jitter()
     {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(3.5, 0.30),
            make_suite(3.8, 0.60),
            make_suite(4.2, 1.20),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("speedup_ratio_p95")),
            "expected clear speedup wins to ignore speedup-ratio suite noise when only a tiny hot-path denominator jitter is inflating the ratio spread"
        );
        assert!(
            report
                .gate
                .warnings
                .iter()
                .all(|warning| !warning.contains("summary_rebuild suite p95")),
            "expected summary_rebuild to stay classified as stable in the hot-denominator jitter case"
        );
    }

    #[test]
    fn memory_context_benchmark_report_keeps_speedup_warning_when_hot_path_spread_is_not_tiny() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(12.0, 2.0),
            make_suite(12.6, 4.0),
            make_suite(13.2, 6.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report
                .gate
                .warnings
                .iter()
                .any(|warning| warning.contains("speedup_ratio_p95")),
            "expected speedup-ratio suite warning to remain visible once hot-path absolute spread is large enough to be operationally meaningful"
        );
    }

    #[test]
    fn memory_context_benchmark_report_qualifies_speedup_warning_when_all_suites_still_clear_the_floor()
     {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |summary_rebuild: f64, summary_steady_state: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &[
                make_suite(12.0, 2.0),
                make_suite(12.2, 4.0),
                make_suite(12.4, 6.0),
            ],
            3,
            false,
            1.10,
        );

        let speedup_warning = report
            .gate
            .warnings
            .iter()
            .find(|warning| warning.contains("speedup_ratio_p95"))
            .expect("expected speedup warning to remain visible for suite noise");
        assert!(
            speedup_warning.contains("every suite still cleared the speedup floor"),
            "expected clear-win suite noise to be qualified instead of described as marginal"
        );
        assert!(
            !speedup_warning.contains("marginal memory context improvements"),
            "expected clear-win suite noise wording to avoid marginal-gain guidance"
        );
    }

    #[test]
    fn memory_context_benchmark_report_warns_when_summary_rebuild_is_suite_noisy() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |summary_rebuild: f64,
             summary_steady_state: f64,
             target_bootstrap_ms: f64,
             target_bootstrap_connection_open_ms: f64,
             target_load_ms: f64,
             copy_db_ms: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![summary_rebuild, summary_rebuild],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![summary_steady_state, summary_steady_state],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![copy_db_ms, copy_db_ms],
                    target_bootstrap_ms: vec![target_bootstrap_ms, target_bootstrap_ms],
                    target_bootstrap_connection_open_ms: vec![
                        target_bootstrap_connection_open_ms,
                        target_bootstrap_connection_open_ms,
                    ],
                    target_load_ms: vec![target_load_ms, target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(4.0, 1.0, 4.0, 4.0, 1.0, 1.0),
            make_suite(8.0, 2.0, 8.0, 8.0, 1.2, 1.2),
            make_suite(12.0, 3.0, 12.0, 12.0, 1.4, 1.4),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );

        assert!(
            report.gate.warnings.iter().any(|warning| {
                warning.contains("summary_rebuild suite p95")
                    && warning.contains("target_bootstrap_ms/connection_open_ms")
            }),
            "expected suite stability warning for noisy summary_rebuild p95"
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_cold_path_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |rebuild_target_load_ms: f64,
             rebuild_copy_db_ms: f64,
             budget_target_load_ms: f64,
             budget_source_warmup_ms: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![rebuild_copy_db_ms, rebuild_copy_db_ms],
                    target_load_ms: vec![rebuild_target_load_ms, rebuild_target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                    source_warmup_ms: vec![budget_source_warmup_ms, budget_source_warmup_ms],
                    target_load_ms: vec![budget_target_load_ms, budget_target_load_ms],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(4.0, 1.0, 2.0, 6.0),
            make_suite(8.0, 1.2, 2.5, 10.0),
            make_suite(12.0, 1.4, 3.0, 14.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_noise_attribution")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("target_load_ms")
        );
        assert_eq!(
            report_json
                .get("cold_path_noise_attribution")
                .and_then(|value| value.get("summary_rebuild_budget_change"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("source_warmup_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_cold_path_bootstrap_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |target_connection_open_ms: f64,
             target_schema_init_ms: f64,
             source_registry_lookup_ms: f64,
             source_schema_upgrade_ms: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_bootstrap_connection_open_ms: vec![
                        target_connection_open_ms,
                        target_connection_open_ms,
                    ],
                    target_bootstrap_schema_init_ms: vec![
                        target_schema_init_ms,
                        target_schema_init_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                    source_bootstrap_registry_lookup_ms: vec![
                        source_registry_lookup_ms,
                        source_registry_lookup_ms,
                    ],
                    source_bootstrap_schema_upgrade_ms: vec![
                        source_schema_upgrade_ms,
                        source_schema_upgrade_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let suite_runs = vec![
            make_suite(4.0, 1.0, 2.0, 6.0),
            make_suite(8.0, 1.2, 2.5, 10.0),
            make_suite(12.0, 1.4, 3.0, 14.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_bootstrap_noise_attribution")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_bootstrap"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("connection_open_ms")
        );
        assert_eq!(
            report_json
                .get("cold_path_bootstrap_noise_attribution")
                .and_then(|value| value.get("summary_rebuild_budget_change"))
                .and_then(|value| value.get("source_bootstrap"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("schema_upgrade_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_cold_path_load_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |budget_target_window_query_ms: f64,
                          budget_target_meta_query_ms: f64,
                          budget_target_update_returning_body_ms: f64,
                          metadata_target_window_query_ms: f64,
                          metadata_target_body_load_ms: f64,
                          metadata_target_catch_up_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_load_window_query_ms: vec![
                        budget_target_window_query_ms,
                        budget_target_window_query_ms,
                    ],
                    target_load_summary_checkpoint_meta_query_ms: vec![
                        budget_target_meta_query_ms,
                        budget_target_meta_query_ms,
                    ],
                    target_load_summary_checkpoint_metadata_update_returning_body_ms: vec![
                        budget_target_update_returning_body_ms,
                        budget_target_update_returning_body_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_load_window_query_ms: vec![
                        metadata_target_window_query_ms,
                        metadata_target_window_query_ms,
                    ],
                    target_load_summary_checkpoint_body_load_ms: vec![
                        metadata_target_body_load_ms,
                        metadata_target_body_load_ms,
                    ],
                    target_load_summary_catch_up_ms: vec![
                        metadata_target_catch_up_ms,
                        metadata_target_catch_up_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(2.0, 3.0, 6.0, 1.0, 2.0, 5.0),
            make_suite(2.5, 3.5, 10.0, 1.2, 2.3, 9.0),
            make_suite(3.0, 4.0, 14.0, 1.4, 2.6, 13.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_load_noise_attribution")
                .and_then(|value| value.get("summary_rebuild_budget_change"))
                .and_then(|value| value.get("target_load"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("summary_checkpoint_metadata_update_returning_body_ms")
        );
        assert_eq!(
            report_json
                .get("cold_path_load_noise_attribution")
                .and_then(|value| value.get("summary_metadata_realign"))
                .and_then(|value| value.get("target_load"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("summary_catch_up_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_split_summary_rebuild_load_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |rebuild_stream_ms: f64,
                          rebuild_metadata_upsert_ms: f64,
                          rebuild_body_upsert_ms: f64,
                          rebuild_commit_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_load_summary_rebuild_ms: vec![1.0, 1.0],
                    target_load_summary_rebuild_stream_ms: vec![
                        rebuild_stream_ms,
                        rebuild_stream_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_metadata_upsert_ms: vec![
                        rebuild_metadata_upsert_ms,
                        rebuild_metadata_upsert_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_body_upsert_ms: vec![
                        rebuild_body_upsert_ms,
                        rebuild_body_upsert_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_commit_ms: vec![
                        rebuild_commit_ms,
                        rebuild_commit_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_upsert_ms: vec![
                        rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                        rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(2.0, 0.3, 0.5, 0.4),
            make_suite(10.0, 0.4, 0.6, 0.5),
            make_suite(14.0, 0.5, 0.7, 0.6),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_load_noise_attribution")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("summary_rebuild_stream_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_split_window_query_load_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |turn_count_ms: f64, known_overflow_rows_ms: f64| MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_load_ms: vec![1.0, 1.0],
                    target_load_window_query_ms: vec![
                        turn_count_ms + known_overflow_rows_ms,
                        turn_count_ms + known_overflow_rows_ms,
                    ],
                    target_load_window_turn_count_query_ms: vec![turn_count_ms, turn_count_ms],
                    target_load_window_known_overflow_rows_query_ms: vec![
                        known_overflow_rows_ms,
                        known_overflow_rows_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            };
        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &[
                make_suite(0.3, 1.0),
                make_suite(0.35, 4.0),
                make_suite(0.4, 8.0),
            ],
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_load_noise_attribution")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("window_known_overflow_rows_query_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_summary_rebuild_checkpoint_commit_noise_attribution() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite = |rebuild_stream_ms: f64,
                          rebuild_metadata_upsert_ms: f64,
                          rebuild_body_upsert_ms: f64,
                          rebuild_commit_ms: f64| {
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![4.0, 4.0],
                summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![2.0, 2.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples {
                    target_load_summary_rebuild_ms: vec![1.0, 1.0],
                    target_load_summary_rebuild_stream_ms: vec![
                        rebuild_stream_ms,
                        rebuild_stream_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_metadata_upsert_ms: vec![
                        rebuild_metadata_upsert_ms,
                        rebuild_metadata_upsert_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_body_upsert_ms: vec![
                        rebuild_body_upsert_ms,
                        rebuild_body_upsert_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_commit_ms: vec![
                        rebuild_commit_ms,
                        rebuild_commit_ms,
                    ],
                    target_load_summary_rebuild_checkpoint_upsert_ms: vec![
                        rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                        rebuild_metadata_upsert_ms + rebuild_body_upsert_ms + rebuild_commit_ms,
                    ],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape,
                summary_window_cover_shape: shape,
                summary_rebuild_shape: shape,
                summary_rebuild_budget_change_shape: shape,
                summary_metadata_realign_shape: shape,
                summary_steady_state_shape: shape,
                window_shrink_catch_up_shape: shape,
            }
        };
        let suite_runs = vec![
            make_suite(0.4, 0.3, 0.4, 1.0),
            make_suite(0.5, 0.35, 0.45, 5.0),
            make_suite(0.6, 0.4, 0.5, 9.0),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            3,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_load_noise_attribution")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load"))
                .and_then(|value| value.get("phase"))
                .and_then(Value::as_str),
            Some("summary_rebuild_checkpoint_commit_ms")
        );
    }

    #[test]
    fn memory_context_benchmark_report_emits_cold_path_phase_reports() {
        let shape = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 64,
            payload_chars: 128,
        };
        let make_suite =
            |rebuild_phase: MemoryContextColdPathPhaseSamples,
             budget_change_phase: MemoryContextColdPathPhaseSamples,
             metadata_phase: MemoryContextColdPathPhaseSamples,
             shrink_phase: MemoryContextColdPathPhaseSamples| {
                MemoryContextBenchmarkSuiteSamples {
                    seed_db_bytes: 512,
                    window_only_samples: vec![1.0, 1.0],
                    summary_window_cover_samples: vec![1.0, 1.0],
                    summary_rebuild_samples: vec![4.0, 4.0],
                    summary_rebuild_budget_change_samples: vec![2.0, 2.0],
                    summary_metadata_realign_samples: vec![1.0, 1.0],
                    summary_steady_state_samples: vec![1.0, 1.0],
                    window_shrink_catch_up_samples: vec![2.0, 2.0],
                    window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                    window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                    summary_append_pre_overflow_samples: vec![1.0, 1.0],
                    summary_append_cold_overflow_samples: vec![1.0, 1.0],
                    summary_append_saturated_samples: vec![1.0, 1.0],
                    window_only_rss_deltas_kib: vec![0.0, 0.0],
                    summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                    summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                    summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                    summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                    summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                    window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                    window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                    window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                    summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                    summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                    summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                    summary_rebuild_phase_samples: rebuild_phase,
                    summary_rebuild_budget_change_phase_samples: budget_change_phase,
                    summary_metadata_realign_phase_samples: metadata_phase,
                    window_shrink_catch_up_phase_samples: shrink_phase,
                    window_only_shape: shape,
                    summary_window_cover_shape: shape,
                    summary_rebuild_shape: shape,
                    summary_rebuild_budget_change_shape: shape,
                    summary_metadata_realign_shape: shape,
                    summary_steady_state_shape: shape,
                    window_shrink_catch_up_shape: shape,
                }
            };
        let suite_runs = vec![
            make_suite(
                MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![1.0, 1.0],
                    target_bootstrap_ms: vec![2.0, 2.0],
                    target_load_ms: vec![3.0, 3.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    source_warmup_ms: vec![5.0, 5.0],
                    target_load_ms: vec![8.0, 8.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    append_turn_ms: vec![1.5, 1.5],
                    target_load_ms: vec![2.0, 2.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    source_warmup_ms: vec![1.0, 1.0],
                    target_load_ms: vec![2.0, 2.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
            ),
            make_suite(
                MemoryContextColdPathPhaseSamples {
                    copy_db_ms: vec![2.0, 2.0],
                    target_bootstrap_ms: vec![4.0, 4.0],
                    target_load_ms: vec![7.0, 7.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    source_warmup_ms: vec![9.0, 9.0],
                    target_load_ms: vec![12.0, 12.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    append_turn_ms: vec![4.5, 4.5],
                    target_load_ms: vec![6.0, 6.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
                MemoryContextColdPathPhaseSamples {
                    source_warmup_ms: vec![3.0, 3.0],
                    target_load_ms: vec![5.0, 5.0],
                    ..MemoryContextColdPathPhaseSamples::default()
                },
            ),
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            2,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        assert_eq!(
            report_json
                .get("cold_path_phases")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load_ms"))
                .and_then(|value| value.get("count"))
                .and_then(Value::as_u64),
            Some(4)
        );
        assert_eq!(
            report_json
                .get("cold_path_phases")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load_ms"))
                .and_then(|value| value.get("p95"))
                .and_then(Value::as_f64),
            Some(7.0)
        );
        assert_eq!(
            report_json
                .get("cold_path_phase_stability")
                .and_then(|value| value.get("summary_rebuild"))
                .and_then(|value| value.get("target_load_ms"))
                .and_then(|value| value.get("range"))
                .and_then(Value::as_f64),
            Some(4.0)
        );
        assert_eq!(
            report_json
                .get("cold_path_phase_stability")
                .and_then(|value| value.get("summary_metadata_realign"))
                .and_then(|value| value.get("append_turn_ms"))
                .and_then(|value| value.get("range"))
                .and_then(Value::as_f64),
            Some(3.0)
        );
    }

    #[test]
    fn memory_context_benchmark_report_tracks_budget_change_workload_adjusted_ratios() {
        let shape_small = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 256,
            payload_chars: 1024,
        };
        let shape_large = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 512,
            payload_chars: 1280,
        };
        let shape_larger = MemoryContextShape {
            entry_count: 2,
            turn_entries: 2,
            summary_chars: 768,
            payload_chars: 1536,
        };
        let suite_runs = vec![
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![2.0, 2.0],
                summary_rebuild_budget_change_samples: vec![3.0, 3.0],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![1.0, 1.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape_small,
                summary_window_cover_shape: shape_small,
                summary_rebuild_shape: shape_small,
                summary_rebuild_budget_change_shape: shape_large,
                summary_metadata_realign_shape: shape_small,
                summary_steady_state_shape: shape_small,
                window_shrink_catch_up_shape: shape_small,
            },
            MemoryContextBenchmarkSuiteSamples {
                seed_db_bytes: 512,
                window_only_samples: vec![1.0, 1.0],
                summary_window_cover_samples: vec![1.0, 1.0],
                summary_rebuild_samples: vec![2.0, 2.0],
                summary_rebuild_budget_change_samples: vec![3.6, 3.6],
                summary_metadata_realign_samples: vec![1.0, 1.0],
                summary_steady_state_samples: vec![1.0, 1.0],
                window_shrink_catch_up_samples: vec![1.0, 1.0],
                window_only_append_pre_overflow_samples: vec![1.0, 1.0],
                window_only_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_pre_overflow_samples: vec![1.0, 1.0],
                summary_append_cold_overflow_samples: vec![1.0, 1.0],
                summary_append_saturated_samples: vec![1.0, 1.0],
                window_only_rss_deltas_kib: vec![0.0, 0.0],
                summary_window_cover_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_budget_change_rss_deltas_kib: vec![0.0, 0.0],
                summary_metadata_realign_rss_deltas_kib: vec![0.0, 0.0],
                summary_steady_state_rss_deltas_kib: vec![0.0, 0.0],
                window_shrink_catch_up_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                window_only_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_pre_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_cold_overflow_rss_deltas_kib: vec![0.0, 0.0],
                summary_append_saturated_rss_deltas_kib: vec![0.0, 0.0],
                summary_rebuild_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                summary_rebuild_budget_change_phase_samples:
                    MemoryContextColdPathPhaseSamples::default(),
                summary_metadata_realign_phase_samples: MemoryContextColdPathPhaseSamples::default(
                ),
                window_shrink_catch_up_phase_samples: MemoryContextColdPathPhaseSamples::default(),
                window_only_shape: shape_small,
                summary_window_cover_shape: shape_small,
                summary_rebuild_shape: shape_small,
                summary_rebuild_budget_change_shape: shape_larger,
                summary_metadata_realign_shape: shape_small,
                summary_steady_state_shape: shape_small,
                window_shrink_catch_up_shape: shape_small,
            },
        ];

        let report = build_memory_context_benchmark_report(
            "target/benchmarks/memory-context-benchmark-report.json",
            &ResolvedMemoryContextBenchmarkTempRoot {
                path: PathBuf::from("target/benchmarks/tmp-local"),
                source: MemoryContextBenchmarkTempRootSource::OutputParent,
            },
            24,
            6,
            12,
            256,
            12,
            2,
            4,
            1,
            &suite_runs,
            2,
            false,
            1.10,
        );
        let report_json = serde_json::to_value(&report).expect("serialize benchmark report");

        let flattened_raw_ratio = report_json
            .get("flattened_sample_ratios")
            .and_then(|value| value.get("summary_rebuild_budget_change_vs_rebuild_ratio_p95"))
            .and_then(Value::as_f64)
            .expect("raw budget-change ratio should be present");
        let flattened_adjusted_ratio = report_json
            .get("flattened_sample_ratios")
            .and_then(|value| {
                value
                    .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            })
            .and_then(Value::as_f64)
            .expect("summary-char-adjusted budget-change ratio should be present");
        let aggregated_adjusted_ratio = report_json
            .get("aggregated_ratios")
            .and_then(|value| {
                value
                    .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95")
            })
            .and_then(Value::as_f64)
            .expect("aggregated summary-char-adjusted budget-change ratio should be present");

        assert!((flattened_raw_ratio - 1.8).abs() < 0.001);
        assert!((flattened_adjusted_ratio - 0.72).abs() < 0.001);
        assert!((aggregated_adjusted_ratio - 0.675).abs() < 0.001);
    }

    #[test]
    fn memory_context_hot_read_helper_excludes_warmup_reads_from_samples() {
        let shape = MemoryContextShape {
            entry_count: 4,
            turn_entries: 4,
            summary_chars: 0,
            payload_chars: 128,
        };
        let mut call_count = 0_usize;

        let (latencies, rss_deltas_kib, final_shape) =
            measure_hot_prompt_context_reads_with_loader(0, 2, false, || {
                call_count = call_count.saturating_add(1);
                Ok(PromptContextReadObservation {
                    latency_ms: call_count as f64,
                    rss_delta_kib: Some((call_count * 10) as f64),
                    shape,
                })
            })
            .expect("hot-read helper should preserve only measured samples");

        assert_eq!(call_count, 3);
        assert_eq!(latencies, vec![2.0, 3.0]);
        assert_eq!(rss_deltas_kib, vec![20.0, 30.0]);
        assert_eq!(final_shape.entry_count, shape.entry_count);
        assert_eq!(final_shape.turn_entries, shape.turn_entries);
        assert_eq!(final_shape.summary_chars, shape.summary_chars);
        assert_eq!(final_shape.payload_chars, shape.payload_chars);
    }

    #[test]
    fn memory_context_hot_read_helper_rejects_missing_summary_during_warmup() {
        let error = measure_hot_prompt_context_reads_with_loader(1, 2, true, || {
            Ok(PromptContextReadObservation {
                latency_ms: 1.0,
                rss_delta_kib: Some(8.0),
                shape: MemoryContextShape {
                    entry_count: 3,
                    turn_entries: 3,
                    summary_chars: 0,
                    payload_chars: 96,
                },
            })
        })
        .expect_err("summary warmup without a summary should fail");

        assert!(error.contains("summary benchmark warmup did not produce a summary entry"));
    }

    #[test]
    fn benchmark_temp_root_uses_unique_suffixes_per_call() {
        let first = benchmark_temp_root("loongclaw-memory-context-benchmark-test", None);
        let second = benchmark_temp_root("loongclaw-memory-context-benchmark-test", None);

        assert_ne!(first, second);
    }

    #[test]
    fn benchmark_copy_strategy_defaults_to_stable_fs_copy() {
        assert_eq!(
            benchmark_copy_strategy_from_env(None),
            BenchmarkCopyStrategy::StableFsCopy
        );
        assert_eq!(
            benchmark_copy_strategy_from_env(Some("".to_owned())),
            BenchmarkCopyStrategy::StableFsCopy
        );
        assert_eq!(
            benchmark_copy_strategy_from_env(Some("copy".to_owned())),
            BenchmarkCopyStrategy::StableFsCopy
        );
        assert_eq!(
            benchmark_copy_strategy_from_env(Some("unexpected".to_owned())),
            BenchmarkCopyStrategy::StableFsCopy
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn benchmark_copy_strategy_accepts_explicit_clone_opt_in() {
        assert_eq!(
            benchmark_copy_strategy_from_env(Some("clone".to_owned())),
            BenchmarkCopyStrategy::MacosCloneCp
        );
        assert_eq!(
            benchmark_copy_strategy_from_env(Some(" CLONE ".to_owned())),
            BenchmarkCopyStrategy::MacosCloneCp
        );
    }

    #[test]
    fn benchmark_temp_root_honors_requested_parent_directory() {
        let requested_parent = Path::new("/tmp/loongclaw-memory-context-benchmark-parent");
        let root = benchmark_temp_root(
            "loongclaw-memory-context-benchmark-test",
            Some(requested_parent),
        );

        assert_eq!(root.parent(), Some(requested_parent));
    }

    #[test]
    fn memory_context_benchmark_temp_root_prefers_explicit_override() {
        let explicit = Path::new("/tmp/loongclaw-memory-context-benchmark-explicit");
        let resolved = resolve_memory_context_benchmark_temp_root(
            "target/benchmarks/memory-context-benchmark-report.json",
            Some(explicit.to_str().expect("utf-8 explicit path")),
        )
        .expect("resolve explicit temp root");

        assert_eq!(resolved.path, explicit);
        assert_eq!(
            resolved.source,
            MemoryContextBenchmarkTempRootSource::Explicit
        );
    }

    #[test]
    fn memory_context_benchmark_temp_root_defaults_to_output_parent_for_target_reports() {
        let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
            "target/benchmarks/memory-context-benchmark-report.json",
            None,
            None,
        )
        .expect("resolve temp root for target benchmark report");

        assert_eq!(resolved.path, Path::new("target/benchmarks/tmp-local"));
        assert_eq!(
            resolved.source,
            MemoryContextBenchmarkTempRootSource::OutputParent
        );
    }

    #[test]
    fn memory_context_benchmark_temp_root_falls_back_to_system_temp_outside_target() {
        let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
            "/tmp/memory-context-benchmark-report.json",
            None,
            None,
        )
        .expect("resolve temp root outside target");

        assert_eq!(resolved.path, std::env::temp_dir());
        assert_eq!(
            resolved.source,
            MemoryContextBenchmarkTempRootSource::SystemTemp
        );
    }

    #[test]
    fn memory_context_benchmark_temp_root_prefers_current_exe_target_dir() {
        let resolved = resolve_memory_context_benchmark_temp_root_with_exe(
            "target/benchmarks/memory-context-benchmark-report.json",
            None,
            Some(Path::new(
                "/repo/target/codex-memory-bench-red/debug/loongclaw",
            )),
        )
        .expect("resolve temp root from current exe");

        assert_eq!(
            resolved.path,
            Path::new("/repo/target/codex-memory-bench-red/tmp-local")
        );
        assert_eq!(
            resolved.source,
            MemoryContextBenchmarkTempRootSource::CurrentExeTargetDir
        );
    }

    #[test]
    fn benchmark_matrix_and_baseline_fixtures_parse() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        assert_eq!(matrix.scenarios.len(), 4);

        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");
        assert!(baseline.scenarios.contains_key("rate_limit_steady_state"));
    }

    #[test]
    fn strict_preflight_reports_missing_spec_run_baseline_entry() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let mut baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        baseline.scenarios.remove("adaptive_concurrency_recovery");

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert_eq!(lint.error_count(), 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind == ProgrammaticPressureBaselineIssueKind::MissingSpecRunBaselineScenario
                && issue.scenario_name == "adaptive_concurrency_recovery"
        }));
    }

    #[test]
    fn strict_preflight_reports_missing_non_spec_run_baseline_entry() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let mut baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        baseline.scenarios.remove("circuit_half_open_transition");

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert_eq!(lint.error_count(), 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind
                == ProgrammaticPressureBaselineIssueKind::MissingMatrixScenarioBaselineThresholds
                && issue.scenario_name == "circuit_half_open_transition"
        }));
    }

    #[test]
    fn strict_preflight_reports_missing_spec_run_schema_fingerprint_field() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let mut baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        let thresholds = baseline
            .scenarios
            .get_mut("rate_limit_steady_state")
            .expect("rate_limit_steady_state baseline should exist");
        thresholds.expected_schema_fingerprint = Some("   ".to_owned());

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert_eq!(lint.error_count(), 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind == ProgrammaticPressureBaselineIssueKind::MissingSpecRunSchemaFingerprint
                && issue.scenario_name == "rate_limit_steady_state"
        }));
    }

    #[test]
    fn strict_preflight_accepts_complete_spec_run_schema_baseline_coverage() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert!(lint.passed());
        assert_eq!(lint.error_count(), 0);
    }

    #[test]
    fn baseline_lint_warns_on_unknown_baseline_scenario() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let mut baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        baseline.scenarios.insert(
            "unknown-extra-scenario".to_owned(),
            ProgrammaticPressureScenarioThresholds::default(),
        );

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert_eq!(lint.error_count(), 0);
        assert_eq!(lint.warning_count(), 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind == ProgrammaticPressureBaselineIssueKind::UnknownBaselineScenario
                && issue.scenario_name == "unknown-extra-scenario"
        }));
    }

    #[test]
    fn baseline_lint_warns_when_non_spec_run_schema_fingerprint_is_configured() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let mut baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        let thresholds = baseline
            .scenarios
            .get_mut("circuit_half_open_transition")
            .expect("circuit_half_open_transition baseline should exist");
        thresholds.expected_schema_fingerprint = Some("ignored-schema".to_owned());

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert_eq!(lint.error_count(), 0);
        assert_eq!(lint.warning_count(), 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind
                == ProgrammaticPressureBaselineIssueKind::NonSpecRunSchemaFingerprintConfigured
                && issue.scenario_name == "circuit_half_open_transition"
        }));
    }

    #[test]
    fn baseline_lint_reports_duplicate_matrix_scenario_name() {
        let matrix_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-matrix.json");
        let mut matrix: ProgrammaticPressureMatrix =
            serde_json::from_str(matrix_raw).expect("matrix fixture must parse");
        let baseline_raw =
            include_str!("../../../examples/benchmarks/programmatic-pressure-baseline.json");
        let baseline: ProgrammaticPressureBaseline =
            serde_json::from_str(baseline_raw).expect("baseline fixture must parse");

        let duplicate = matrix
            .scenarios
            .first()
            .expect("matrix should have first scenario")
            .clone();
        matrix.scenarios.push(duplicate);

        let lint = lint_programmatic_pressure_baseline(&matrix, &baseline);
        assert!(lint.error_count() >= 1);
        assert!(lint.issues.iter().any(|issue| {
            issue.kind == ProgrammaticPressureBaselineIssueKind::DuplicateMatrixScenarioName
                && issue.scenario_name == "rate_limit_steady_state"
        }));
    }

    #[test]
    fn lint_gate_can_optionally_fail_on_warnings() {
        let lint = ProgrammaticPressureBaselineLintResult {
            issues: vec![ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Warning,
                kind: ProgrammaticPressureBaselineIssueKind::UnknownBaselineScenario,
                scenario_name: "unknown-extra-scenario".to_owned(),
                message: "baseline scenario does not exist in matrix".to_owned(),
            }],
        };

        assert!(lint_gate_passed(&lint, false));
        assert!(!lint_gate_passed(&lint, true));
    }

    #[test]
    fn build_preflight_distinguishes_passed_and_gate_passed() {
        let lint = ProgrammaticPressureBaselineLintResult {
            issues: vec![ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Warning,
                kind: ProgrammaticPressureBaselineIssueKind::UnknownBaselineScenario,
                scenario_name: "unknown-extra-scenario".to_owned(),
                message: "baseline scenario does not exist in matrix".to_owned(),
            }],
        };

        let preflight_without_warning_block = build_baseline_preflight(&lint, true, false);
        assert!(preflight_without_warning_block.passed);
        assert!(preflight_without_warning_block.gate_passed);
        assert_eq!(preflight_without_warning_block.warning_count, 1);
        assert_eq!(preflight_without_warning_block.error_count, 0);
        assert!(!preflight_without_warning_block.fail_on_warnings);

        let preflight_with_warning_block = build_baseline_preflight(&lint, true, true);
        assert!(preflight_with_warning_block.passed);
        assert!(!preflight_with_warning_block.gate_passed);
        assert!(preflight_with_warning_block.fail_on_warnings);
    }

    #[test]
    fn preflight_gate_issue_filter_respects_warning_policy() {
        let issues = vec![
            ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Error,
                kind: ProgrammaticPressureBaselineIssueKind::MissingSpecRunSchemaFingerprint,
                scenario_name: "spec-a".to_owned(),
                message: "missing fingerprint".to_owned(),
            },
            ProgrammaticPressureBaselineIssue {
                severity: ProgrammaticPressureBaselineIssueSeverity::Warning,
                kind: ProgrammaticPressureBaselineIssueKind::UnknownBaselineScenario,
                scenario_name: "warning-only".to_owned(),
                message: "unknown scenario".to_owned(),
            },
        ];

        let errors_only = preflight_gate_issues(&issues, false);
        assert_eq!(errors_only.len(), 1);
        assert_eq!(errors_only[0].scenario_name, "spec-a");

        let include_warnings = preflight_gate_issues(&issues, true);
        assert_eq!(include_warnings.len(), 2);
    }

    #[test]
    fn schema_fingerprint_is_stable_for_same_shape() {
        let left = json!({
            "step_outputs": {
                "fanout": {
                    "calls": [
                        {"call_id":"a","status":"ok","execution":{"attempts":1,"retries":0}},
                        {"call_id":"b","status":"error","error_code":"connector_execution_error"}
                    ],
                    "scheduler": {
                        "peak_in_flight": 2,
                        "budget_reductions": 1
                    }
                }
            }
        });
        let right = json!({
            "step_outputs": {
                "fanout": {
                    "calls": [
                        {"call_id":"x","status":"ok","execution":{"attempts":9,"retries":3}},
                        {"call_id":"y","status":"error","error_code":"circuit_open"}
                    ],
                    "scheduler": {
                        "peak_in_flight": 8,
                        "budget_reductions": 0
                    }
                }
            }
        });
        let changed_shape = json!({
            "step_outputs": {
                "fanout": {
                    "calls": [
                        {"call_id":"z","status":"ok","execution":{"attempts":1,"new_field":true}}
                    ]
                }
            }
        });

        let left_hash =
            extract_programmatic_schema_fingerprint(&left).expect("left fingerprint should exist");
        let right_hash = extract_programmatic_schema_fingerprint(&right)
            .expect("right fingerprint should exist");
        let changed_hash = extract_programmatic_schema_fingerprint(&changed_shape)
            .expect("changed fingerprint should exist");
        assert_eq!(left_hash, right_hash);
        assert_ne!(left_hash, changed_hash);
    }

    #[test]
    fn scenario_gate_fails_when_threshold_regresses() {
        let report = ProgrammaticPressureScenarioReport {
            name: "adaptive".to_owned(),
            description: None,
            scenario_kind: "spec_run".to_owned(),
            iterations: 10,
            warmup_iterations: 1,
            success_runs: 8,
            failed_runs: 2,
            blocked_runs: 0,
            error_rate: 0.2,
            blocked_rate: 0.0,
            connector_calls_total: 120,
            throughput_rps: 12.0,
            latency_ms: NumericStats {
                count: 10,
                min: Some(10.0),
                max: Some(30.0),
                avg: Some(20.0),
                p50: Some(20.0),
                p95: Some(29.0),
                p99: Some(29.8),
            },
            circuit_open_error_ratio: 0.0,
            schema_fingerprint: Some("schema-a".to_owned()),
            scheduler: Some(ProgrammaticSchedulerAggregate {
                observed_runs: 10,
                peak_in_flight_max: 3,
                peak_in_flight_avg: 2.4,
                budget_reductions_total: 3,
                budget_increases_total: 2,
                wait_cycles_total: 5,
                min_final_in_flight_budget: 1,
                max_final_in_flight_budget: 3,
            }),
            circuit: None,
            error_codes: BTreeMap::new(),
            gate: ProgrammaticPressureScenarioGate {
                passed: true,
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let thresholds = ProgrammaticPressureScenarioThresholds {
            max_error_rate: Some(0.1),
            max_p95_latency_ms: Some(50.0),
            max_p99_latency_ms: Some(60.0),
            min_throughput_rps: Some(5.0),
            min_peak_in_flight: Some(2.0),
            max_circuit_open_error_ratio: Some(0.2),
            max_half_open_p95_ms: None,
            expected_schema_fingerprint: None,
            tolerance: ProgrammaticPressureGateTolerance::default(),
        };

        let gate = evaluate_scenario_gate(&report, Some(&thresholds), true);
        assert!(!gate.passed);
        assert!(
            gate.checks
                .iter()
                .any(|check| { check.metric == "error_rate" && !check.passed })
        );
    }

    #[test]
    fn scenario_gate_fails_on_schema_fingerprint_mismatch() {
        let report = ProgrammaticPressureScenarioReport {
            name: "schema".to_owned(),
            description: None,
            scenario_kind: "spec_run".to_owned(),
            iterations: 4,
            warmup_iterations: 1,
            success_runs: 4,
            failed_runs: 0,
            blocked_runs: 0,
            error_rate: 0.0,
            blocked_rate: 0.0,
            connector_calls_total: 8,
            throughput_rps: 16.0,
            latency_ms: NumericStats {
                count: 4,
                min: Some(10.0),
                max: Some(20.0),
                avg: Some(14.0),
                p50: Some(13.0),
                p95: Some(19.0),
                p99: Some(19.8),
            },
            circuit_open_error_ratio: 0.0,
            schema_fingerprint: Some("observed-schema".to_owned()),
            scheduler: None,
            circuit: None,
            error_codes: BTreeMap::new(),
            gate: ProgrammaticPressureScenarioGate {
                passed: true,
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let thresholds = ProgrammaticPressureScenarioThresholds {
            max_error_rate: Some(0.0),
            max_p95_latency_ms: Some(100.0),
            max_p99_latency_ms: Some(100.0),
            min_throughput_rps: Some(1.0),
            min_peak_in_flight: None,
            max_circuit_open_error_ratio: Some(0.2),
            max_half_open_p95_ms: None,
            expected_schema_fingerprint: Some("expected-schema".to_owned()),
            tolerance: ProgrammaticPressureGateTolerance::default(),
        };

        let gate = evaluate_scenario_gate(&report, Some(&thresholds), true);
        let schema_check = gate
            .checks
            .iter()
            .find(|check| check.metric == "schema_fingerprint")
            .expect("schema check should be present");
        assert!(!schema_check.passed);
        assert!(
            schema_check
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("expected=expected-schema")
        );
    }

    #[test]
    fn scenario_gate_fails_when_spec_run_schema_fingerprint_baseline_missing_in_strict_mode() {
        let report = ProgrammaticPressureScenarioReport {
            name: "schema-missing-baseline".to_owned(),
            description: None,
            scenario_kind: "spec_run".to_owned(),
            iterations: 4,
            warmup_iterations: 1,
            success_runs: 4,
            failed_runs: 0,
            blocked_runs: 0,
            error_rate: 0.0,
            blocked_rate: 0.0,
            connector_calls_total: 8,
            throughput_rps: 16.0,
            latency_ms: NumericStats {
                count: 4,
                min: Some(10.0),
                max: Some(20.0),
                avg: Some(14.0),
                p50: Some(13.0),
                p95: Some(19.0),
                p99: Some(19.8),
            },
            circuit_open_error_ratio: 0.0,
            schema_fingerprint: Some("observed-schema".to_owned()),
            scheduler: None,
            circuit: None,
            error_codes: BTreeMap::new(),
            gate: ProgrammaticPressureScenarioGate {
                passed: true,
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let thresholds = ProgrammaticPressureScenarioThresholds {
            max_error_rate: Some(0.0),
            max_p95_latency_ms: Some(100.0),
            max_p99_latency_ms: Some(100.0),
            min_throughput_rps: Some(1.0),
            min_peak_in_flight: None,
            max_circuit_open_error_ratio: Some(0.2),
            max_half_open_p95_ms: None,
            expected_schema_fingerprint: None,
            tolerance: ProgrammaticPressureGateTolerance::default(),
        };

        let gate = evaluate_scenario_gate(&report, Some(&thresholds), true);
        let schema_check = gate
            .checks
            .iter()
            .find(|check| check.metric == "schema_fingerprint_baseline_presence")
            .expect("schema baseline presence check should be present");
        assert!(!schema_check.passed);
    }

    #[test]
    fn scenario_gate_warns_when_spec_run_schema_fingerprint_baseline_missing_non_strict() {
        let report = ProgrammaticPressureScenarioReport {
            name: "schema-missing-baseline".to_owned(),
            description: None,
            scenario_kind: "spec_run".to_owned(),
            iterations: 4,
            warmup_iterations: 1,
            success_runs: 4,
            failed_runs: 0,
            blocked_runs: 0,
            error_rate: 0.0,
            blocked_rate: 0.0,
            connector_calls_total: 8,
            throughput_rps: 16.0,
            latency_ms: NumericStats {
                count: 4,
                min: Some(10.0),
                max: Some(20.0),
                avg: Some(14.0),
                p50: Some(13.0),
                p95: Some(19.0),
                p99: Some(19.8),
            },
            circuit_open_error_ratio: 0.0,
            schema_fingerprint: Some("observed-schema".to_owned()),
            scheduler: None,
            circuit: None,
            error_codes: BTreeMap::new(),
            gate: ProgrammaticPressureScenarioGate {
                passed: true,
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let thresholds = ProgrammaticPressureScenarioThresholds {
            max_error_rate: Some(0.0),
            max_p95_latency_ms: Some(100.0),
            max_p99_latency_ms: Some(100.0),
            min_throughput_rps: Some(1.0),
            min_peak_in_flight: None,
            max_circuit_open_error_ratio: Some(0.2),
            max_half_open_p95_ms: None,
            expected_schema_fingerprint: None,
            tolerance: ProgrammaticPressureGateTolerance::default(),
        };

        let gate = evaluate_scenario_gate(&report, Some(&thresholds), false);
        assert!(gate.passed);
        assert!(
            gate.warnings
                .iter()
                .any(|warning| warning.contains("expected_schema_fingerprint missing"))
        );
    }

    #[test]
    fn scenario_gate_tolerance_allows_expected_runtime_jitter() {
        let report = ProgrammaticPressureScenarioReport {
            name: "drift".to_owned(),
            description: None,
            scenario_kind: "spec_run".to_owned(),
            iterations: 10,
            warmup_iterations: 1,
            success_runs: 10,
            failed_runs: 0,
            blocked_runs: 0,
            error_rate: 0.0,
            blocked_rate: 0.0,
            connector_calls_total: 120,
            throughput_rps: 92.0,
            latency_ms: NumericStats {
                count: 10,
                min: Some(90.0),
                max: Some(112.0),
                avg: Some(101.0),
                p50: Some(100.0),
                p95: Some(111.0),
                p99: Some(112.0),
            },
            circuit_open_error_ratio: 0.0,
            schema_fingerprint: Some("schema-a".to_owned()),
            scheduler: Some(ProgrammaticSchedulerAggregate {
                observed_runs: 10,
                peak_in_flight_max: 2,
                peak_in_flight_avg: 2.0,
                budget_reductions_total: 0,
                budget_increases_total: 0,
                wait_cycles_total: 0,
                min_final_in_flight_budget: 2,
                max_final_in_flight_budget: 2,
            }),
            circuit: None,
            error_codes: BTreeMap::new(),
            gate: ProgrammaticPressureScenarioGate {
                passed: true,
                checks: Vec::new(),
                warnings: Vec::new(),
            },
        };

        let thresholds = ProgrammaticPressureScenarioThresholds {
            max_error_rate: Some(0.0),
            max_p95_latency_ms: Some(100.0),
            max_p99_latency_ms: Some(100.0),
            min_throughput_rps: Some(100.0),
            min_peak_in_flight: Some(2.0),
            max_circuit_open_error_ratio: Some(0.1),
            max_half_open_p95_ms: None,
            expected_schema_fingerprint: Some("schema-a".to_owned()),
            tolerance: ProgrammaticPressureGateTolerance {
                max_ratio: 0.05,
                min_ratio: 0.10,
                latency_ms: 10.0,
            },
        };

        let gate = evaluate_scenario_gate(&report, Some(&thresholds), true);
        assert!(gate.passed);
        assert!(gate.checks.iter().all(|check| check.passed));

        let throughput_check = gate
            .checks
            .iter()
            .find(|check| check.metric == "throughput_rps")
            .expect("throughput check should exist");
        assert_eq!(throughput_check.baseline_threshold, Some(100.0));
        assert!((throughput_check.threshold - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn normalized_gate_tolerance_clamps_invalid_values() {
        let normalized = normalized_gate_tolerance(&ProgrammaticPressureGateTolerance {
            max_ratio: f64::INFINITY,
            min_ratio: -0.5,
            latency_ms: f64::NAN,
        });
        assert_eq!(normalized.max_ratio, 0.0);
        assert_eq!(normalized.min_ratio, 0.0);
        assert_eq!(normalized.latency_ms, 0.0);
    }
}
