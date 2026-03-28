use kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
use loongclaw_bench::test_support::*;
use loongclaw_spec::spec_requires_native_tool_executor;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

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
        plugin_setup_readiness: None,
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
        plugin_setup_readiness: None,
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
        plugin_setup_readiness: None,
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
        plugin_setup_readiness: None,
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
