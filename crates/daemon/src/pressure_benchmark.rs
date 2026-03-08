use super::*;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

const DEFAULT_PRESSURE_ITERATIONS: usize = 12;
const DEFAULT_PRESSURE_WARMUP_ITERATIONS: usize = 2;
const DEFAULT_CIRCUIT_POLL_INTERVAL_MS: u64 = 5;
const DEFAULT_CIRCUIT_RECOVERY_BUFFER_MS: u64 = 250;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ProgrammaticPressureMatrix {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgrammaticPressureScenario {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    iterations: Option<usize>,
    #[serde(default)]
    warmup_iterations: Option<usize>,
    #[serde(default = "default_pressure_expected_operation_kind")]
    expected_operation_kind: String,
    #[serde(default)]
    allow_blocked: bool,
    #[serde(flatten)]
    kind: ProgrammaticPressureScenarioKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ProgrammaticPressureScenarioKind {
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
struct ScenarioRunSample {
    latency_ms: f64,
    passed: bool,
    blocked: bool,
    connector_calls: usize,
    error_codes: BTreeMap<String, usize>,
    schema_fingerprint: Option<String>,
    scheduler: Option<SchedulerSnapshot>,
    half_open_transition_ms: Option<f64>,
    closed_after_recovery: Option<bool>,
}

#[derive(Debug, Clone)]
struct SchedulerSnapshot {
    peak_in_flight: usize,
    final_in_flight_budget: usize,
    budget_reductions: usize,
    budget_increases: usize,
    wait_cycles: usize,
}

pub(super) async fn run_programmatic_pressure_benchmark_cli(
    matrix_path: &str,
    baseline_path: Option<&str>,
    output_path: &str,
    enforce_gate: bool,
    preflight_fail_on_warnings: bool,
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
    if enforce_gate {
        if let Some(preflight_summary) = preflight.as_ref() {
            if !preflight_summary.gate_passed {
                let gate_issues = preflight_gate_issues(
                    &preflight_summary.issues,
                    preflight_summary.fail_on_warnings,
                );
                return Err(format!(
                    "programmatic pressure strict preflight failed: {}",
                    format_baseline_issue_list(&gate_issues)
                ));
            }
        }
    }

    let report = run_programmatic_pressure_matrix(
        &matrix,
        matrix_path,
        selected_baseline_path.as_deref(),
        baseline.as_ref(),
        preflight,
        enforce_gate,
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

pub(super) fn run_programmatic_pressure_baseline_lint_cli(
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
        baseline_path: baseline_path.clone(),
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
        issues: lint.issues.clone(),
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

async fn run_programmatic_pressure_matrix(
    matrix: &ProgrammaticPressureMatrix,
    matrix_path: &str,
    baseline_path: Option<&str>,
    baseline: Option<&ProgrammaticPressureBaseline>,
    preflight: Option<ProgrammaticPressureBaselinePreflight>,
    enforce_gate: bool,
) -> ProgrammaticPressureReport {
    let mut scenarios = Vec::new();
    for scenario in &matrix.scenarios {
        let baseline_thresholds = baseline.and_then(|value| value.scenarios.get(&scenario.name));
        let report =
            run_programmatic_pressure_scenario(matrix, scenario, baseline_thresholds, enforce_gate)
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

    for (scenario_name, _thresholds) in &baseline.scenarios {
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
) -> ProgrammaticPressureScenarioReport {
    let iterations = scenario
        .iterations
        .unwrap_or(matrix.default_iterations)
        .max(1);
    let warmup_iterations = scenario
        .warmup_iterations
        .unwrap_or(matrix.default_warmup_iterations);

    for _ in 0..warmup_iterations {
        let _ = run_pressure_scenario_once(scenario).await;
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = TokioInstant::now();
        let run = run_pressure_scenario_once(scenario).await;
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
) -> CliResult<ScenarioRunSample> {
    match &scenario.kind {
        ProgrammaticPressureScenarioKind::SpecRun { spec } => {
            run_spec_pressure_once(spec, scenario).await
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

async fn run_spec_pressure_once(
    spec: &RunnerSpec,
    scenario: &ProgrammaticPressureScenario,
) -> CliResult<ScenarioRunSample> {
    let report = execute_spec(spec.clone(), false).await;
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
    format!("{:x}", hasher.finalize())
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
        return sorted_values[0];
    }

    let clamped = ratio.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len().saturating_sub(1) as f64);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted_values[lower];
    }

    let weight = rank - lower as f64;
    sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * weight
}

fn read_json_file<T: DeserializeOwned>(path: &str) -> CliResult<T> {
    let raw =
        fs::read_to_string(path).map_err(|error| format!("failed to read {path}: {error}"))?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to parse {path}: {error}"))
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
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
        assert!(gate
            .checks
            .iter()
            .any(|check| { check.metric == "error_rate" && !check.passed }));
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
        assert!(schema_check
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("expected=expected-schema"));
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
        assert!(gate
            .warnings
            .iter()
            .any(|warning| warning.contains("expected_schema_fingerprint missing")));
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
