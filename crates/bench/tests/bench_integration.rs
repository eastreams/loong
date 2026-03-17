use kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
use loongclaw_bench::run_memory_context_benchmark_cli;
use loongclaw_bench::test_support::*;
use loongclaw_spec::spec_requires_native_tool_executor;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

// ---------------------------------------------------------------------------
// Filesystem integration tests
// ---------------------------------------------------------------------------

#[test]
fn memory_context_benchmark_rejects_history_not_exceeding_window() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-invalid.json");

    let error = run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        None,
        8,
        8,
        256,
        8,
        2,
        4,
        1,
        1,
        false,
        1.10,
    )
    .expect_err("history equal to window should be rejected");

    assert!(error.contains("history_turns must exceed sliding_window"));

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn memory_context_benchmark_rejects_history_without_shrink_catch_up_headroom() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-shrink-invalid-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-shrink-invalid.json");

    let error = run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        None,
        9,
        8,
        256,
        8,
        2,
        4,
        1,
        1,
        false,
        1.10,
    )
    .expect_err("history with only one turn beyond the window should be rejected");

    assert!(error.contains("shrink catch-up mode"));

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn memory_context_benchmark_writes_report_with_all_scenarios() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-memory-context-benchmark-report-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let output = tmp.join("memory-context-benchmark-report.json");
    let temp_root = tmp.join("benchmark-temp-root");

    run_memory_context_benchmark_cli(
        output.to_str().expect("utf-8 output path"),
        Some(temp_root.to_str().expect("utf-8 temp root path")),
        24,
        6,
        256,
        12,
        2,
        4,
        1,
        2,
        false,
        1.10,
    )
    .expect("memory context benchmark should write report");

    let report_raw = fs::read_to_string(&output).expect("read benchmark report");
    let report: Value = serde_json::from_str(&report_raw).expect("benchmark report JSON");

    assert_eq!(report.get("history_turns"), Some(&json!(24)));
    assert_eq!(report.get("suite_repetitions"), Some(&json!(2)));
    assert_eq!(
        report.get("suite_aggregation"),
        Some(&json!("median_of_suite_p95"))
    );
    assert_eq!(
        report.get("rss_telemetry_scope"),
        Some(&json!("best_effort_approx_process_rss_step_delta_via_ps"))
    );
    assert_eq!(
        report.get("benchmark_temp_root"),
        Some(&json!(temp_root.display().to_string()))
    );
    assert_eq!(
        report.get("benchmark_temp_root_source"),
        Some(&json!("explicit"))
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("summary_steady_state"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("speedup_ratio_p95"))
            .is_some()
    );
    assert!(report.get("window_only_latency_ms").is_some());
    assert!(report.get("summary_window_cover_latency_ms").is_some());
    assert!(report.get("summary_rebuild_latency_ms").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_latency_ms")
            .is_some()
    );
    assert!(report.get("summary_metadata_realign_latency_ms").is_some());
    assert!(report.get("summary_steady_state_latency_ms").is_some());
    assert!(report.get("window_shrink_catch_up_latency_ms").is_some());
    assert!(
        report
            .get("window_only_append_pre_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("window_only_append_cold_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_pre_overflow_latency_ms")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_cold_overflow_latency_ms")
            .is_some()
    );
    assert!(report.get("summary_append_saturated_latency_ms").is_some());
    assert!(report.get("window_only_rss_delta_kib").is_some());
    assert!(report.get("summary_window_cover_rss_delta_kib").is_some());
    assert!(report.get("summary_rebuild_rss_delta_kib").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_metadata_realign_rss_delta_kib")
            .is_some()
    );
    assert!(report.get("summary_steady_state_rss_delta_kib").is_some());
    assert!(report.get("window_shrink_catch_up_rss_delta_kib").is_some());
    assert!(
        report
            .get("window_only_append_pre_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("window_only_append_cold_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_pre_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_cold_overflow_rss_delta_kib")
            .is_some()
    );
    assert!(
        report
            .get("summary_append_saturated_rss_delta_kib")
            .is_some()
    );
    assert!(report.get("window_only_payload_chars").is_some());
    assert!(report.get("summary_window_cover_payload_chars").is_some());
    assert!(report.get("summary_rebuild_payload_chars").is_some());
    assert!(
        report
            .get("summary_rebuild_budget_change_payload_chars")
            .is_some()
    );
    assert!(
        report
            .get("summary_metadata_realign_payload_chars")
            .is_some()
    );
    assert!(report.get("summary_steady_state_payload_chars").is_some());
    assert!(report.get("window_shrink_catch_up_payload_chars").is_some());
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_window_cover_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_window_cover_overhead_p95_ms"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_rebuild_budget_change_vs_rebuild_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v
                .get("summary_rebuild_budget_change_vs_rebuild_summary_char_adjusted_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_metadata_realign_vs_budget_change_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("speedup_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("window_shrink_catch_up_vs_rebuild_speedup_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_append_pre_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("flattened_sample_ratios")
            .and_then(|v| v.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("window_only_append_pre_overflow"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_p95_median_ms")
            .and_then(|v| v.get("window_only_append_cold_overflow"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("summary_append_pre_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("aggregated_ratios")
            .and_then(|v| v.get("summary_append_cold_overflow_vs_window_only_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_max_overhead_p95_ms"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_window_cover_soft_warning_min_samples"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_rebuild_budget_change_vs_rebuild_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("summary_metadata_realign_vs_budget_change_soft_max_ratio_p95"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("suite_stability_soft_warning_min_suites"))
            .is_some()
    );
    assert!(
        report
            .get("gate")
            .and_then(|g| g.get("suite_stability_soft_max_range_over_p50"))
            .is_some()
    );
    assert!(report.get("gate").and_then(|g| g.get("warnings")).is_some());

    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn benchmark_copy_helper_preserves_contents() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-benchmark-copy-helper-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let source = tmp.join("source.sqlite3");
    let destination = tmp.join("destination.sqlite3");
    let payload = b"benchmark-copy-helper-payload";
    fs::write(&source, payload).expect("write source payload");

    copy_benchmark_file(&source, &destination).expect("copy benchmark file");

    assert_eq!(
        fs::read(&destination).expect("read copied payload"),
        payload.as_slice()
    );

    let _ = fs::remove_dir_all(&tmp);
}

// ---------------------------------------------------------------------------
// Async integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_spec_pressure_once_rejects_native_claw_migrate_scenarios() {
    let spec = loongclaw_spec::RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "bench-spec-native-claw-migrate".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        },
        agent_id: "bench-agent-native-claw-migrate".to_owned(),
        ttl_s: 60,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: loongclaw_spec::OperationSpec::ToolCore {
            tool_name: "claw.migrate".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"mode": "plan"}),
            core: None,
        },
    };
    let scenario = ProgrammaticPressureScenario {
        name: "native-claw-migrate".to_owned(),
        description: None,
        iterations: Some(1),
        warmup_iterations: Some(0),
        expected_operation_kind: "tool_core".to_owned(),
        allow_blocked: false,
        kind: ProgrammaticPressureScenarioKind::SpecRun { spec: spec.clone() },
    };

    let error = run_spec_pressure_once(&spec, &scenario, None)
        .await
        .expect_err("bench spec runs should reject native claw.migrate scenarios");
    assert!(error.contains("native tool executor"));
}

#[tokio::test]
async fn run_spec_pressure_once_uses_native_executor_when_present() {
    let spec = loongclaw_spec::RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "bench-spec-native-claw-migrate-exec".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        },
        agent_id: "bench-agent-native-claw-migrate-exec".to_owned(),
        ttl_s: 60,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: loongclaw_spec::OperationSpec::ToolCore {
            tool_name: "claw.migrate".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"mode": "plan"}),
            core: None,
        },
    };
    let scenario = ProgrammaticPressureScenario {
        name: "native-claw-migrate-exec".to_owned(),
        description: None,
        iterations: Some(1),
        warmup_iterations: Some(0),
        expected_operation_kind: "tool_core".to_owned(),
        allow_blocked: false,
        kind: ProgrammaticPressureScenarioKind::SpecRun { spec: spec.clone() },
    };

    let sample = run_spec_pressure_once(&spec, &scenario, Some(test_native_tool_executor))
        .await
        .expect("bench spec runs should execute when a native executor is injected");

    assert!(sample.passed);
    assert!(!sample.blocked);
}

#[tokio::test]
async fn run_spec_pressure_once_errors_when_executor_declines_native_request() {
    let spec = loongclaw_spec::RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "bench-spec-native-claw-migrate-declined".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        },
        agent_id: "bench-agent-native-claw-migrate-declined".to_owned(),
        ttl_s: 60,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: loongclaw_spec::OperationSpec::ToolCore {
            tool_name: "claw.migrate".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"mode": "plan"}),
            core: None,
        },
    };
    let scenario = ProgrammaticPressureScenario {
        name: "native-claw-migrate-declined".to_owned(),
        description: None,
        iterations: Some(1),
        warmup_iterations: Some(0),
        expected_operation_kind: "tool_core".to_owned(),
        allow_blocked: true,
        kind: ProgrammaticPressureScenarioKind::SpecRun { spec: spec.clone() },
    };

    let error = run_spec_pressure_once(&spec, &scenario, Some(declining_native_tool_executor))
        .await
        .expect_err("bench spec runs should error when executor declines native requests");

    assert!(error.contains("native tool executor"));
}

// ---------------------------------------------------------------------------
// Spec extension detection (cross-crate integration)
// ---------------------------------------------------------------------------

#[test]
fn spec_requires_native_tool_executor_detects_claw_migration_extension() {
    let spec = loongclaw_spec::RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "bench-spec-claw-migration-extension".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        },
        agent_id: "bench-agent-claw-migration-extension".to_owned(),
        ttl_s: 60,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: loongclaw_spec::OperationSpec::ToolExtension {
            extension_action: "plan".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"input_path": "/tmp/demo"}),
            extension: "claw-migration".to_owned(),
            core: None,
        },
    };

    assert!(spec_requires_native_tool_executor(&spec));
}
