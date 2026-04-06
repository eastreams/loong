use kernel::{Capability, ExecutionRoute, HarnessKind, VerticalPackManifest};
use loongclaw_bench::test_support::*;
use loongclaw_spec::spec_requires_native_tool_executor;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

fn native_config_import_spec(tool_name: &str, suffix: &str) -> loongclaw_spec::RunnerSpec {
    let pack_id = format!("bench-spec-native-config-import-{suffix}");
    let agent_id = format!("bench-agent-native-config-import-{suffix}");

    loongclaw_spec::RunnerSpec {
        pack: VerticalPackManifest {
            pack_id,
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
        agent_id,
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
            tool_name: tool_name.to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"mode": "plan"}),
            core: None,
        },
    }
}

fn native_config_import_scenario(
    scenario_name: String,
    spec: &loongclaw_spec::RunnerSpec,
    allow_blocked: bool,
) -> ProgrammaticPressureScenario {
    ProgrammaticPressureScenario {
        name: scenario_name,
        description: None,
        iterations: Some(1),
        warmup_iterations: Some(0),
        expected_operation_kind: "tool_core".to_owned(),
        allow_blocked,
        kind: ProgrammaticPressureScenarioKind::SpecRun { spec: spec.clone() },
    }
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
async fn run_spec_pressure_once_rejects_native_config_import_scenarios() {
    let cases = [
        ("config.import", "canonical"),
        ("claw.migrate", "legacy-alias"),
        ("claw_migrate", "legacy-underscore-alias"),
    ];

    for (tool_name, case_name) in cases {
        let spec = native_config_import_spec(tool_name, case_name);
        let scenario_name = format!("native-config-import-{case_name}");
        let scenario = native_config_import_scenario(scenario_name, &spec, false);
        let error = run_spec_pressure_once(&spec, &scenario, None)
            .await
            .expect_err("bench spec runs should reject native config.import scenarios");

        assert!(
            error.contains("native tool executor"),
            "tool `{tool_name}` should require a native tool executor: {error}"
        );
    }
}

#[tokio::test]
async fn run_spec_pressure_once_uses_native_executor_when_present() {
    let cases = [
        ("config.import", "exec-canonical"),
        ("claw.migrate", "exec-legacy-alias"),
        ("claw_migrate", "exec-legacy-underscore-alias"),
    ];

    for (tool_name, case_name) in cases {
        let spec = native_config_import_spec(tool_name, case_name);
        let scenario_name = format!("native-config-import-{case_name}");
        let scenario = native_config_import_scenario(scenario_name, &spec, false);
        let sample = run_spec_pressure_once(&spec, &scenario, Some(test_native_tool_executor))
            .await
            .expect("bench spec runs should execute when a native executor is injected");

        assert!(sample.passed, "tool `{tool_name}` should pass");
        assert!(!sample.blocked, "tool `{tool_name}` should not be blocked");
    }
}

#[tokio::test]
async fn run_spec_pressure_once_errors_when_executor_declines_native_request() {
    let cases = [
        ("config.import", "declined-canonical"),
        ("claw.migrate", "declined-legacy-alias"),
        ("claw_migrate", "declined-legacy-underscore-alias"),
    ];

    for (tool_name, case_name) in cases {
        let spec = native_config_import_spec(tool_name, case_name);
        let scenario_name = format!("native-config-import-{case_name}");
        let scenario = native_config_import_scenario(scenario_name, &spec, true);
        let error = run_spec_pressure_once(&spec, &scenario, Some(declining_native_tool_executor))
            .await
            .expect_err("bench spec runs should error when executor declines native requests");

        assert!(
            error.contains("native tool executor"),
            "tool `{tool_name}` should surface the native executor failure: {error}"
        );
    }
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
