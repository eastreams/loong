use super::*;
use loongclaw_daemon::kernel::{
    PluginActivationStatus, PluginBridgeKind, PluginCompatibilityMode, PluginCompatibilityShim,
    PluginCompatibilityShimSupport, PluginContractDialect,
};

fn render_cli_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn template_spec_is_json_roundtrip_stable() {
    let spec = RunnerSpec::template();
    let encoded = serde_json::to_string_pretty(&spec).expect("encode spec");
    let decoded: RunnerSpec = serde_json::from_str(&encoded).expect("decode spec");
    assert_eq!(decoded.pack.pack_id, "sales-intel-local");
    let readiness = decoded
        .plugin_setup_readiness
        .expect("template should expose plugin setup readiness");
    assert!(readiness.inherit_process_env);
    assert!(readiness.verified_env_vars.is_empty());
    assert!(readiness.verified_config_keys.is_empty());
    assert!(matches!(
        decoded.operation,
        OperationSpec::RuntimeExtension { .. }
    ));
}

#[test]
fn runtime_extension_fixture_uses_backward_compatible_spec_defaults() {
    let raw = include_str!("../../../../examples/spec/runtime-extension.json")
        .replace("\n  \"hotfixes\": [],", "");
    let parsed: RunnerSpec = serde_json::from_str(&raw)
        .expect("runtime-extension fixture should parse when hotfixes is omitted");
    assert!(parsed.hotfixes.is_empty());
    assert!(parsed.plugin_setup_readiness.is_none());
}

#[test]
fn tool_search_trusted_fixture_parses_structured_trust_tiers() {
    let raw = include_str!("../../../../examples/spec/tool-search-trusted.json");
    let parsed: RunnerSpec =
        serde_json::from_str(raw).expect("tool-search-trusted fixture should parse");

    let OperationSpec::ToolSearch { trust_tiers, .. } = parsed.operation else {
        panic!("tool-search-trusted fixture should use tool_search operation");
    };
    assert_eq!(trust_tiers.len(), 1);
    assert_eq!(trust_tiers[0].as_str(), "official");
}

#[test]
fn init_spec_cli_plugin_trust_guard_preset_writes_expected_bootstrap_defaults() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("loongclaw-plugin-trust-guard-{unique}.json"));

    init_spec_cli(
        path.to_str().expect("temp path should be utf8"),
        InitSpecPreset::PluginTrustGuard,
    )
    .expect("init-spec should write trust guard preset");

    let raw = fs::read_to_string(&path).expect("read generated spec");
    let parsed: RunnerSpec = serde_json::from_str(&raw).expect("generated spec should parse");
    let plugin_scan = parsed
        .plugin_scan
        .as_ref()
        .expect("trust guard preset should enable plugin scan");
    let bridge_support = parsed
        .bridge_support
        .as_ref()
        .expect("trust guard preset should enable bridge support");
    let bootstrap = parsed
        .bootstrap
        .as_ref()
        .expect("trust guard preset should enable bootstrap");

    assert_eq!(parsed.pack.pack_id, "community-plugin-intake");
    assert_eq!(plugin_scan.roots, vec!["plugins"]);
    assert!(bridge_support.enabled);
    assert!(bridge_support.enforce_supported);
    assert!(
        bridge_support
            .supported_bridges
            .contains(&PluginBridgeKind::ProcessStdio)
    );
    assert_eq!(bootstrap.allow_process_stdio_auto_apply, Some(true));
    assert_eq!(bootstrap.block_unverified_high_risk_auto_apply, Some(true));
    assert_eq!(bootstrap.enforce_ready_execution, Some(true));

    fs::remove_file(path).expect("remove generated spec");
}

#[test]
fn example_spec_fixtures_parse_as_runner_specs() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/spec");
    let mut parsed_specs = 0usize;

    for entry in fs::read_dir(&examples_dir).expect("read example spec directory") {
        let entry = entry.expect("read example spec entry");
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read_to_string(&path).expect("read example spec file");
        serde_json::from_str::<RunnerSpec>(&raw).unwrap_or_else(|error| {
            panic!(
                "example spec fixture should parse: {}: {error}",
                path.display()
            )
        });
        parsed_specs = parsed_specs.saturating_add(1);
    }

    assert!(
        parsed_specs >= 1,
        "expected at least one example spec fixture under {}",
        examples_dir.display()
    );
}

#[test]
fn run_spec_cli_render_summary_preserves_stdout_json_and_writes_trust_search_summary() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let spec_path = workspace_root.join("examples/spec/tool-search-trusted.json");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .arg("run-spec")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--render-summary")
        .current_dir(&workspace_root)
        .output()
        .expect("spawn run-spec with render-summary");
    let stdout = render_cli_output(&output.stdout);
    let stderr = render_cli_output(&output.stderr);

    assert!(
        output.status.success(),
        "run-spec should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should remain machine-readable JSON");
    assert_eq!(payload["pack_id"], "tool-search-trusted-pack");
    assert_eq!(
        payload["tool_search_summary"]["headline"],
        "query=\"echo\"; returned 1 result; trust_scope=official; filtered_out=2 candidates; top_match=wasm-secure-echo"
    );
    assert!(
        !stdout.contains("run-spec summary"),
        "stdout should not include human summary lines: {stdout:?}"
    );
    assert!(
        stderr.contains(
            "run-spec summary pack=tool-search-trusted-pack agent=agent-tool-search-trusted status=ok operation=tool_search"
        ),
        "stderr should include top-level run summary: {stderr:?}"
    );
    assert!(
        stderr.contains(
            "plugin_trust scanned=3 official=1 verified_community=1 unverified=1 high_risk=3 high_risk_unverified=1 blocked_auto_apply=0 review_required=1"
        ),
        "stderr should include plugin trust rollup: {stderr:?}"
    );
    assert!(
        stderr.contains(
            "tool_search query=\"echo\"; returned 1 result; trust_scope=official; filtered_out=2 candidates; top_match=wasm-secure-echo"
        ),
        "stderr should include the trust-aware tool-search headline: {stderr:?}"
    );
    assert!(
        stderr.contains(
            "tool_search_filters query_requested=- structured_requested=official effective=official conflicting=false filtered_out_by_tier=unverified:1,verified-community:1"
        ),
        "stderr should include the resolved trust filter breakdown: {stderr:?}"
    );
    assert!(
        stderr.contains(
            "tool_search_top[1] provider=wasm-secure-echo connector=wasm-secure-echo tool_id=wasm-secure-echo::wasm-secure-echo trust=official bridge=wasm_component"
        ),
        "stderr should include a compact top-result card: {stderr:?}"
    );
}

#[test]
fn run_spec_cli_render_summary_surfaces_blocked_plugin_trust_review() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let spec_path = workspace_root.join("examples/spec/plugin-bootstrap-trust-policy.json");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .arg("run-spec")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--render-summary")
        .current_dir(&workspace_root)
        .output()
        .expect("spawn blocked run-spec with render-summary");
    let stdout = render_cli_output(&output.stdout);
    let stderr = render_cli_output(&output.stderr);

    assert!(
        output.status.success(),
        "blocked run-spec should still serialize a report, stdout={stdout:?}, stderr={stderr:?}"
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("stdout should remain machine-readable JSON");
    assert_eq!(payload["operation_kind"], "blocked");
    assert_eq!(payload["outcome"]["status"], "blocked");
    assert!(
        stderr.contains(
            "run-spec summary pack=plugin-bootstrap-trust-policy-pack agent=agent-plugin-bootstrap-trust-policy status=blocked operation=blocked"
        ),
        "stderr should mark the blocked run clearly: {stderr:?}"
    );
    assert!(
        stderr.contains("blocked_reason=bootstrap policy blocked"),
        "stderr should include the blocked reason: {stderr:?}"
    );
    assert!(
        stderr.contains("plugin_trust scanned=1 official=0 verified_community=0 unverified=1 high_risk=1 high_risk_unverified=1 blocked_auto_apply=1 review_required=1"),
        "stderr should include the trust-policy rollup: {stderr:?}"
    );
    assert!(
        stderr.contains("plugin_review plugin=stdio-echo-py tier=unverified bridge=process_stdio activation=ready bootstrap=deferred_unsupported_auto_apply"),
        "stderr should include the review-required plugin entry: {stderr:?}"
    );
}

#[tokio::test]
async fn plugin_bootstrap_trust_policy_fixture_blocks_unverified_process_plugin() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/spec/plugin-bootstrap-trust-policy.json");
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let raw = fs::read_to_string(&fixture_path).expect("read trust policy fixture");
    let mut spec: RunnerSpec =
        serde_json::from_str(&raw).expect("trust policy fixture should parse");

    let plugin_scan = spec
        .plugin_scan
        .as_mut()
        .expect("trust policy fixture should define plugin scan");
    for root in &mut plugin_scan.roots {
        *root = workspace_root.join(root.as_str()).display().to_string();
    }

    let report = execute_spec(&spec, true).await;

    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("bootstrap policy blocked")
    );
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 0);
    assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
    assert_eq!(
        report.plugin_bootstrap_reports[0].tasks[0].trust_tier,
        loongclaw_daemon::kernel::PluginTrustTier::Unverified
    );
    assert_eq!(report.plugin_trust_summary.scanned_plugins, 1);
    assert_eq!(report.plugin_trust_summary.unverified_plugins, 1);
    assert_eq!(report.plugin_trust_summary.high_risk_plugins, 1);
    assert_eq!(report.plugin_trust_summary.high_risk_unverified_plugins, 1);
    assert_eq!(report.plugin_trust_summary.blocked_auto_apply_plugins, 1);
    assert_eq!(report.plugin_trust_summary.review_required_plugins.len(), 1);
    assert_eq!(
        report.plugin_trust_summary.review_required_plugins[0]
            .bootstrap_status
            .expect("bootstrap status should exist"),
        loongclaw_daemon::kernel::BootstrapTaskStatus::DeferredUnsupportedAutoApply
    );
    let audit = report.audit_events.expect("audit events should exist");
    assert!(audit.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::PluginTrustEvaluated {
                scanned_plugins,
                high_risk_unverified_plugins,
                blocked_auto_apply_plugins,
                review_required_plugin_ids,
                review_required_bridges,
                ..
            } if *scanned_plugins == 1
                && *high_risk_unverified_plugins == 1
                && *blocked_auto_apply_plugins == 1
                && review_required_plugin_ids == &vec!["stdio-echo-py".to_owned()]
                && review_required_bridges == &vec!["process_stdio".to_owned()]
        )
    }));
}

#[test]
fn read_spec_file_materializes_relative_bridge_support_delta_selection() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-delta-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let delta_path = root.join("bridge-support.delta.json");
    let spec_path = root.join("runner.spec.json");

    let delta_artifact = materialize_bridge_support_delta_artifact(
        "openclaw-ecosystem-balanced",
        Some(&PluginPreflightBridgeProfileDelta {
            supported_bridges: Vec::new(),
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: Vec::new(),
            supported_compatibility_shims: Vec::new(),
            shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                shim_id: "openclaw-modern-compat".to_owned(),
                shim_family: "openclaw-modern-compat".to_owned(),
                supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                supported_bridges: vec!["process_stdio".to_owned()],
                supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                supported_source_languages: vec!["python".to_owned()],
            }],
            unresolved_blocking_reasons: Vec::new(),
        }),
    )
    .expect("delta artifact should materialize");
    let materialized = materialize_bridge_support_template(
        "openclaw-ecosystem-balanced",
        Some(&delta_artifact.delta),
    )
    .expect("delta template should materialize");

    fs::write(
        &delta_path,
        serde_json::to_string_pretty(&delta_artifact).expect("serialize delta artifact"),
    )
    .expect("write delta artifact");

    let mut spec_value = serde_json::to_value(RunnerSpec::template()).expect("encode template");
    spec_value["bridge_support_selection"] = json!({
        "delta_artifact": "bridge-support.delta.json",
        "expected_delta_sha256": delta_artifact.sha256,
        "expected_sha256": materialized.sha256
    });
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec_value).expect("serialize spec file"),
    )
    .expect("write spec file");

    let resolved = read_spec_file_with_bridge_support_resolution(
        spec_path.to_str().expect("spec path should be utf-8"),
        None,
    )
    .expect("spec file should parse");
    let expected_bridge_support_source = format!("delta:{}", delta_path.display());
    let expected_delta_source = delta_path.display().to_string();
    assert_eq!(
        resolved.bridge_support_source.as_deref(),
        Some(expected_bridge_support_source.as_str())
    );
    assert_eq!(
        resolved.bridge_support_delta_source.as_deref(),
        Some(expected_delta_source.as_str())
    );
    assert_eq!(
        resolved.bridge_support_delta_sha256.as_deref(),
        Some(delta_artifact.sha256.as_str())
    );
    let bridge_support = resolved
        .spec
        .bridge_support
        .expect("bridge support should materialize from delta selection");
    assert_eq!(
        bridge_support.policy_version.as_deref(),
        Some("custom-derived-from-openclaw-ecosystem-balanced")
    );
    assert!(
        bridge_support
            .supported_compatibility_shim_profiles
            .iter()
            .any(|profile| {
                profile.shim.shim_id == "openclaw-modern-compat"
                    && profile.supported_source_languages.contains("python")
            })
    );
}

#[test]
fn read_spec_file_rejects_inline_bridge_support_and_selection_mix() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-bridge-mix-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let spec_path = root.join("runner.spec.json");

    let mut spec_value = serde_json::to_value(RunnerSpec::template()).expect("encode template");
    spec_value["bridge_support"] = json!({
        "enabled": true,
        "supported_bridges": ["process_stdio"],
        "supported_adapter_families": [],
        "supported_compatibility_modes": ["native"],
        "supported_compatibility_shims": [],
        "supported_compatibility_shim_profiles": [],
        "enforce_supported": true,
        "policy_version": "inline-test",
        "expected_checksum": null,
        "expected_sha256": null,
        "execute_process_stdio": true,
        "execute_http_json": false,
        "allowed_process_commands": ["node"],
        "enforce_execution_success": false,
        "security_scan": null
    });
    spec_value["bridge_support_selection"] = json!({
        "bundled_profile": "native-balanced"
    });
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec_value).expect("serialize spec file"),
    )
    .expect("write spec file");

    let error = read_spec_file(spec_path.to_str().expect("spec path should be utf-8"))
        .expect_err("mixed inline bridge support and selection should fail");
    assert!(error.contains("bridge_support_selection"));
    assert!(error.contains("not both"));
}

#[test]
fn read_spec_file_accepts_cli_bridge_support_selection_override_when_file_has_none() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-cli-delta-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let delta_path = root.join("bridge-support.delta.json");
    let spec_path = root.join("runner.spec.json");

    let delta_artifact = materialize_bridge_support_delta_artifact(
        "openclaw-ecosystem-balanced",
        Some(&PluginPreflightBridgeProfileDelta {
            supported_bridges: Vec::new(),
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: Vec::new(),
            supported_compatibility_shims: Vec::new(),
            shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                shim_id: "openclaw-modern-compat".to_owned(),
                shim_family: "openclaw-modern-compat".to_owned(),
                supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                supported_bridges: vec!["process_stdio".to_owned()],
                supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                supported_source_languages: vec!["python".to_owned()],
            }],
            unresolved_blocking_reasons: Vec::new(),
        }),
    )
    .expect("delta artifact should materialize");
    fs::write(
        &delta_path,
        serde_json::to_string_pretty(&delta_artifact).expect("serialize delta artifact"),
    )
    .expect("write delta artifact");
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&RunnerSpec::template()).expect("serialize spec"),
    )
    .expect("write spec");

    let resolved = read_spec_file_with_bridge_support_resolution(
        spec_path.to_str().expect("spec path should be utf-8"),
        Some(&BridgeSupportSelectionInput {
            path: None,
            bundled_profile: None,
            delta_artifact: Some(delta_path.display().to_string()),
            expected_sha256: None,
            expected_delta_sha256: Some(delta_artifact.sha256.clone()),
        }),
    )
    .expect("spec file should accept CLI bridge support selection override");
    let expected_bridge_support_source = format!("delta:{}", delta_path.display());
    let expected_delta_source = delta_path.display().to_string();
    assert_eq!(
        resolved.bridge_support_source.as_deref(),
        Some(expected_bridge_support_source.as_str())
    );
    assert_eq!(
        resolved.bridge_support_delta_source.as_deref(),
        Some(expected_delta_source.as_str())
    );
    assert_eq!(
        resolved.bridge_support_delta_sha256.as_deref(),
        Some(delta_artifact.sha256.as_str())
    );

    let bridge_support = resolved
        .spec
        .bridge_support
        .expect("bridge support should materialize from CLI override");
    assert_eq!(
        bridge_support.policy_version.as_deref(),
        Some("custom-derived-from-openclaw-ecosystem-balanced")
    );
    assert!(
        bridge_support
            .supported_compatibility_shim_profiles
            .iter()
            .any(|profile| {
                profile.shim.shim_id == "openclaw-modern-compat"
                    && profile.supported_source_languages.contains("python")
            })
    );
}

#[test]
fn read_spec_file_surfaces_inline_bridge_support_provenance() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-inline-bridge-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let spec_path = root.join("runner.spec.json");

    let mut spec_value = serde_json::to_value(RunnerSpec::template()).expect("encode template");
    spec_value["bridge_support"] = json!({
        "enabled": true,
        "supported_bridges": ["process_stdio"],
        "supported_adapter_families": [],
        "supported_compatibility_modes": ["native"],
        "supported_compatibility_shims": [],
        "supported_compatibility_shim_profiles": [],
        "enforce_supported": true,
        "policy_version": "inline-test",
        "expected_checksum": null,
        "expected_sha256": null,
        "execute_process_stdio": true,
        "execute_http_json": false,
        "allowed_process_commands": ["node"],
        "enforce_execution_success": false,
        "security_scan": null
    });
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec_value).expect("serialize spec file"),
    )
    .expect("write spec file");

    let resolved = read_spec_file_with_bridge_support_resolution(
        spec_path.to_str().expect("spec path should be utf-8"),
        None,
    )
    .expect("inline bridge support should parse");
    let expected_source = format!("inline:{}", spec_path.display());

    assert_eq!(
        resolved.bridge_support_source.as_deref(),
        Some(expected_source.as_str())
    );
    assert!(resolved.bridge_support_delta_source.is_none());
    assert!(resolved.bridge_support_delta_sha256.is_none());
}

#[test]
fn run_spec_cli_emits_bridge_support_provenance_in_final_report() {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-report-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let delta_path = root.join("bridge-support.delta.json");
    let spec_path = root.join("runner.spec.json");

    let delta_artifact = materialize_bridge_support_delta_artifact(
        "openclaw-ecosystem-balanced",
        Some(&PluginPreflightBridgeProfileDelta {
            supported_bridges: Vec::new(),
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: Vec::new(),
            supported_compatibility_shims: Vec::new(),
            shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                shim_id: "openclaw-modern-compat".to_owned(),
                shim_family: "openclaw-modern-compat".to_owned(),
                supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                supported_bridges: vec!["process_stdio".to_owned()],
                supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                supported_source_languages: vec!["python".to_owned()],
            }],
            unresolved_blocking_reasons: Vec::new(),
        }),
    )
    .expect("delta artifact should materialize");
    let materialized = materialize_bridge_support_template(
        "openclaw-ecosystem-balanced",
        Some(&delta_artifact.delta),
    )
    .expect("delta template should materialize");
    fs::write(
        &delta_path,
        serde_json::to_string_pretty(&delta_artifact).expect("serialize delta artifact"),
    )
    .expect("write delta artifact");

    let mut spec = RunnerSpec::template();
    spec.auto_provision = None;
    spec.operation = OperationSpec::ConnectorLegacy {
        connector_name: "non-existent".to_owned(),
        operation: "notify".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({}),
    };
    let mut spec_value = serde_json::to_value(spec).expect("encode spec");
    spec_value["bridge_support_selection"] = json!({
        "delta_artifact": "bridge-support.delta.json",
        "expected_delta_sha256": delta_artifact.sha256,
        "expected_sha256": materialized.sha256
    });
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec_value).expect("serialize spec file"),
    )
    .expect("write spec file");

    let output = Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .args(["run-spec", "--spec"])
        .arg(&spec_path)
        .current_dir(&root)
        .output()
        .expect("run-spec cli should execute");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(
        output.status.success(),
        "run-spec should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );

    let report: Value =
        serde_json::from_str(&stdout).expect("run-spec stdout should be a json report");
    assert_eq!(
        report["schema_version"].as_u64(),
        Some(SPEC_RUN_REPORT_SCHEMA_VERSION as u64)
    );
    assert_eq!(
        report["schema"]["version"].as_u64(),
        Some(SPEC_RUN_REPORT_SCHEMA_VERSION as u64)
    );
    assert_eq!(
        report["schema"]["surface"].as_str(),
        Some("spec_run_report")
    );
    assert_eq!(
        report["schema"]["purpose"].as_str(),
        Some("runtime_execution")
    );
    let expected_bridge_support_source = format!("delta:{}", delta_path.display());
    let expected_delta_source = delta_path.display().to_string();
    assert_eq!(
        report["bridge_support_source"].as_str(),
        Some(expected_bridge_support_source.as_str())
    );
    assert_eq!(
        report["bridge_support_delta_source"].as_str(),
        Some(expected_delta_source.as_str())
    );
    assert_eq!(
        report["bridge_support_delta_sha256"].as_str(),
        Some(delta_artifact.sha256.as_str())
    );
    assert_eq!(
        report["bridge_support_checksum"].as_str(),
        Some(materialized.checksum.as_str())
    );
    assert_eq!(
        report["bridge_support_sha256"].as_str(),
        Some(materialized.sha256.as_str())
    );
}

#[test]
fn run_spec_cli_resolves_bridge_support_delta_override_relative_to_process_cwd() {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("loongclaw-run-spec-cli-relative-bridge-{unique}"));
    let spec_dir = root.join("configs");
    fs::create_dir_all(&spec_dir).expect("create spec dir");
    let delta_path = root.join("bridge-support.delta.json");
    let spec_path = spec_dir.join("runner.spec.json");

    let delta_artifact = materialize_bridge_support_delta_artifact(
        "openclaw-ecosystem-balanced",
        Some(&PluginPreflightBridgeProfileDelta {
            supported_bridges: Vec::new(),
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: Vec::new(),
            supported_compatibility_shims: Vec::new(),
            shim_profile_additions: vec![PluginPreflightBridgeShimProfileDelta {
                shim_id: "openclaw-modern-compat".to_owned(),
                shim_family: "openclaw-modern-compat".to_owned(),
                supported_dialects: vec!["openclaw_modern_manifest".to_owned()],
                supported_bridges: vec!["process_stdio".to_owned()],
                supported_adapter_families: vec!["openclaw-modern-compat".to_owned()],
                supported_source_languages: vec!["python".to_owned()],
            }],
            unresolved_blocking_reasons: Vec::new(),
        }),
    )
    .expect("delta artifact should materialize");
    fs::write(
        &delta_path,
        serde_json::to_string_pretty(&delta_artifact).expect("serialize delta artifact"),
    )
    .expect("write delta artifact");

    let mut spec = RunnerSpec::template();
    spec.auto_provision = None;
    spec.operation = OperationSpec::ConnectorLegacy {
        connector_name: "non-existent".to_owned(),
        operation: "notify".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({}),
    };
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&spec).expect("serialize spec"),
    )
    .expect("write spec");

    let output = Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .args([
            "run-spec",
            "--spec",
            "configs/runner.spec.json",
            "--bridge-support-delta",
            "bridge-support.delta.json",
            "--bridge-support-delta-sha256",
            delta_artifact.sha256.as_str(),
        ])
        .current_dir(&root)
        .output()
        .expect("run-spec cli should execute");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(
        output.status.success(),
        "run-spec should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );

    let report: Value =
        serde_json::from_str(&stdout).expect("run-spec stdout should be a json report");
    let expected_delta_source = fs::canonicalize(&delta_path)
        .expect("delta path should canonicalize")
        .display()
        .to_string();
    let expected_bridge_support_source = format!("delta:{expected_delta_source}");

    assert_eq!(
        report["bridge_support_delta_source"].as_str(),
        Some(expected_delta_source.as_str())
    );
    assert_eq!(
        report["bridge_support_source"].as_str(),
        Some(expected_bridge_support_source.as_str())
    );
}

#[test]
fn run_spec_cli_rejects_bridge_support_sha256_pins_without_policy_source() {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("loongclaw-run-spec-sha-only-{unique}"));
    fs::create_dir_all(&root).expect("create temp root");
    let spec_path = root.join("runner.spec.json");

    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&RunnerSpec::template()).expect("serialize spec"),
    )
    .expect("write spec");

    let output = Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .args([
            "run-spec",
            "--spec",
            spec_path.to_str().expect("spec path should be utf-8"),
            "--bridge-support-sha256",
            "abc123",
        ])
        .current_dir(&root)
        .output()
        .expect("run-spec cli should execute");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert!(!output.status.success(), "run-spec should fail");
    assert!(stderr.contains("bridge support sha256 pins require"));
}

#[tokio::test]
async fn execute_spec_returns_blocked_instead_of_panicking_on_operation_error() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-blocked-op".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-blocked-op".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "non-existent".to_owned(),
            operation: "notify".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("legacy connector execution from spec failed")
    );
}

#[test]
fn approval_uses_external_risk_profile_without_inline_overrides() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("loongclaw-risk-profile-{unique}.json"));
    write_temp_risk_profile(
        &path,
        r#"{
  "high_risk_keywords": ["irrelevant"],
  "high_risk_tool_names": ["irrelevant-tool"],
  "high_risk_payload_keys": ["irrelevant_key"],
  "scoring": {
    "keyword_weight": 10,
    "tool_name_weight": 10,
    "payload_key_weight": 10,
    "keyword_hit_cap": 2,
    "payload_key_hit_cap": 2,
    "high_risk_threshold": 10
  }
}"#,
    );

    let policy = HumanApprovalSpec {
        risk_profile_path: Some(path.display().to_string()),
        ..HumanApprovalSpec::default()
    };
    let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
    let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

    assert_eq!(risk_level, ApprovalRiskLevel::Low);
    assert!(matched.is_empty());
    assert_eq!(score, 0);
}

#[test]
fn approval_inline_risk_signals_override_external_profile() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("loongclaw-risk-profile-override-{unique}.json"));
    write_temp_risk_profile(
        &path,
        r#"{
  "high_risk_keywords": ["irrelevant"],
  "high_risk_tool_names": ["irrelevant-tool"],
  "high_risk_payload_keys": ["irrelevant_key"],
  "scoring": {
    "keyword_weight": 10,
    "tool_name_weight": 10,
    "payload_key_weight": 10,
    "keyword_hit_cap": 2,
    "payload_key_hit_cap": 2,
    "high_risk_threshold": 10
  }
}"#,
    );

    let policy = HumanApprovalSpec {
        risk_profile_path: Some(path.display().to_string()),
        high_risk_tool_names: vec!["delete-file".to_owned()],
        ..HumanApprovalSpec::default()
    };
    let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
    let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

    assert_eq!(risk_level, ApprovalRiskLevel::High);
    assert!(matched.iter().any(|value| value == "tool:delete-file"));
    assert_eq!(score, 10);
}

#[test]
fn approval_falls_back_to_bundled_profile_when_path_missing() {
    let policy = HumanApprovalSpec {
        risk_profile_path: Some("/tmp/loongclaw-risk-profile-missing.json".to_owned()),
        ..HumanApprovalSpec::default()
    };
    let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
    let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

    assert_eq!(risk_level, ApprovalRiskLevel::High);
    assert!(matched.iter().any(|value| value == "tool:delete-file"));
    assert!(score >= 20);
}

#[test]
fn security_scan_profile_path_overrides_bundled_defaults() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("loongclaw-security-profile-{unique}.json"));
    fs::write(
        &path,
        r#"{
  "high_risk_metadata_keywords": ["custom-danger-keyword"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 123456,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi-custom"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
    )
    .expect("write security scan profile");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-profile-path".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-profile-path".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: Some(path.display().to_string()),
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-profile-path".to_owned(),
            objective: "verify profile loading".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let policy = security_scan_policy(&spec)
        .expect("security scan policy should resolve")
        .expect("security scan policy should be enabled");
    assert_eq!(
        policy.high_risk_metadata_keywords,
        vec!["custom-danger-keyword".to_owned()]
    );
    assert_eq!(policy.wasm.max_module_bytes, 123456);
    assert_eq!(
        policy.wasm.blocked_import_prefixes,
        vec!["wasi-custom".to_owned()]
    );
}

#[test]
fn security_scan_profile_sha256_pin_accepts_matching_profile() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loongclaw-security-profile-sha-match-{unique}.json"
    ));
    fs::write(
        &path,
        r#"{
  "high_risk_metadata_keywords": ["pinned-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 654321,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi-custom"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
    )
    .expect("write pinned profile");

    let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
        .expect("profile should load");
    let profile_sha256 = security_scan_profile_sha256(&profile);

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-profile-pin".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-profile-pin".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: Some(path.display().to_string()),
                profile_sha256: Some(profile_sha256),
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-profile-pin".to_owned(),
            objective: "verify profile sha pin".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let policy = security_scan_policy(&spec)
        .expect("security scan policy should resolve")
        .expect("security scan policy should be enabled");
    assert_eq!(
        policy.high_risk_metadata_keywords,
        vec!["pinned-danger".to_owned()]
    );
    assert_eq!(policy.wasm.max_module_bytes, 654321);
}

#[tokio::test]
async fn execute_spec_blocks_when_security_scan_profile_sha256_mismatches() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loongclaw-security-profile-sha-mismatch-{unique}.json"
    ));
    fs::write(
        &path,
        r#"{
  "high_risk_metadata_keywords": ["mismatch-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 1024,
    "allow_wasi": false,
    "blocked_import_prefixes": [],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
    )
    .expect("write mismatched profile");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-profile-mismatch".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-profile-mismatch".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: Some(path.display().to_string()),
                profile_sha256: Some("deadbeef".repeat(8)),
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-profile-mismatch".to_owned(),
            objective: "mismatch pin should block".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("profile sha256 mismatch")
    );
}

#[test]
fn security_scan_profile_signature_accepts_matching_signature() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loongclaw-security-profile-signature-match-{unique}.json"
    ));
    fs::write(
        &path,
        r#"{
  "high_risk_metadata_keywords": ["signed-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 2048,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
    )
    .expect("write signed profile");

    let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
        .expect("profile should load");
    let (public_key_base64, signature_base64) = sign_security_scan_profile_for_test(&profile);

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-signature-pin".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-signature-pin".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: Some(path.display().to_string()),
                profile_sha256: None,
                profile_signature: Some(SecurityProfileSignatureSpec {
                    algorithm: "ed25519".to_owned(),
                    public_key_base64,
                    signature_base64,
                }),
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-signature-pin".to_owned(),
            objective: "verify profile signature pin".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let policy = security_scan_policy(&spec)
        .expect("security scan policy should resolve")
        .expect("security scan policy should be enabled");
    assert_eq!(
        policy.high_risk_metadata_keywords,
        vec!["signed-danger".to_owned()]
    );
}

#[tokio::test]
async fn execute_spec_blocks_when_security_scan_profile_signature_mismatches() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loongclaw-security-profile-signature-mismatch-{unique}.json"
    ));
    fs::write(
        &path,
        r#"{
  "high_risk_metadata_keywords": ["signed-mismatch"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 1024,
    "allow_wasi": false,
    "blocked_import_prefixes": [],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
    )
    .expect("write signed mismatch profile");

    let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
        .expect("profile should load");
    let (public_key_base64, mut signature_base64) = sign_security_scan_profile_for_test(&profile);
    let replacement = if signature_base64.starts_with('A') {
        "B"
    } else {
        "A"
    };
    signature_base64.replace_range(0..1, replacement);

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-signature-mismatch".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-signature-mismatch".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: Some(path.display().to_string()),
                profile_sha256: None,
                profile_signature: Some(SecurityProfileSignatureSpec {
                    algorithm: "ed25519".to_owned(),
                    public_key_base64,
                    signature_base64,
                }),
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-signature-mismatch".to_owned(),
            objective: "signature mismatch should block".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("profile signature verification failed")
    );
}

#[tokio::test]
async fn execute_spec_runs_runtime_extension_and_captures_audit() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-test-pack".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["crm".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-spec-test".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: Some(DefaultCoreSelection {
            connector: None,
            runtime: Some("fallback-core".to_owned()),
            tool: None,
            memory: None,
        }),
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::RuntimeExtension {
            action: "start".to_owned(),
            required_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            payload: json!({}),
            extension: "acp-bridge".to_owned(),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "runtime_extension");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    let events = report.audit_events.expect("audit should be included");
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            kernel::AuditEventKind::PlaneInvoked {
                plane: kernel::ExecutionPlane::Runtime,
                tier: kernel::PlaneTier::Extension,
                ..
            }
        )
    }));
}

#[tokio::test]
async fn execute_spec_auto_provisions_provider_and_channel_when_missing() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-auto-provision".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-auto".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: Some(AutoProvisionSpec {
            enabled: true,
            provider_id: "openrouter".to_owned(),
            channel_id: "primary".to_owned(),
            connector_name: Some("openrouter".to_owned()),
            endpoint: Some("https://openrouter.ai/api/v1/chat/completions".to_owned()),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        }),
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "openrouter".to_owned(),
            operation: "chat".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["provider_id"],
        "openrouter"
    );
    assert!(report.auto_provision_plan.is_some());
    assert!(report.integration_catalog.provider("openrouter").is_some());
    assert!(report.integration_catalog.channel("primary").is_some());
}

#[tokio::test]
async fn execute_spec_applies_hotfix_endpoint_before_invocation() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-hotfix".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-hotfix".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: Some(AutoProvisionSpec {
            enabled: true,
            provider_id: "slack".to_owned(),
            channel_id: "alerts".to_owned(),
            connector_name: Some("slack".to_owned()),
            endpoint: Some("https://old.slack.invalid/hook".to_owned()),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        }),
        hotfixes: vec![HotfixSpec::ChannelEndpoint {
            channel_id: "alerts".to_owned(),
            new_endpoint: "https://hooks.slack.com/services/new".to_owned(),
        }],
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "slack".to_owned(),
            operation: "notify".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"channel_id": "alerts"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(
        report.outcome["outcome"]["payload"]["endpoint"],
        "https://hooks.slack.com/services/new"
    );
}

#[tokio::test]
async fn execute_spec_scans_plugin_files_and_absorbs_them_for_hotplug() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-plugin-{}", unique));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let plugin_file = plugin_root.join("openrouter_plugin.rs");
    fs::write(
        &plugin_file,
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0","source":"community"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin file");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-scan".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-scan".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "openrouter".to_owned(),
            operation: "chat".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.plugin_scan_reports.len(), 1);
    assert_eq!(report.plugin_scan_reports[0].matched_plugins, 1);
    assert_eq!(report.plugin_translation_reports.len(), 1);
    assert_eq!(report.plugin_translation_reports[0].translated_plugins, 1);
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].ready_plugins, 1);
    assert_eq!(report.plugin_bootstrap_queue.len(), 1);
    assert_eq!(report.plugin_absorb_reports.len(), 1);
    assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
    assert!(report.integration_catalog.provider("openrouter").is_some());
    assert!(report.integration_catalog.channel("primary").is_some());
}

#[tokio::test]
async fn execute_spec_blocks_when_bridge_matrix_does_not_support_plugin() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-plugin-bridge-{}", unique));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let plugin_file = plugin_root.join("openrouter_plugin.rs");
    fs::write(
        &plugin_file,
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0","source":"community"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin file");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-bridge-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-bridge-block".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "openrouter".to_owned(),
            operation: "chat".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 1);
    assert!(report.plugin_bootstrap_queue.is_empty());
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(report.integration_catalog.provider("openrouter").is_none());
}

#[tokio::test]
async fn execute_spec_skips_blocked_plugins_when_bridge_enforcement_is_disabled() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-bridge-selective-{}", unique));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let rust_plugin = plugin_root.join("openrouter.rs");
    fs::write(
        &rust_plugin,
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write rust plugin");

    let http_plugin = plugin_root.join("webhook.js");
    fs::write(
        &http_plugin,
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "webhook-js",
//   "provider_id": "webhookx",
//   "connector_name": "webhookx",
//   "channel_id": "primary",
//   "endpoint": "https://hooks.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-bridge-selective".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-bridge-selective".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: false,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "webhookx".to_owned(),
            operation: "notify".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].ready_plugins, 1);
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 1);
    assert_eq!(report.plugin_bootstrap_queue.len(), 1);
    assert_eq!(report.plugin_absorb_reports.len(), 1);
    assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
    assert!(report.integration_catalog.provider("webhookx").is_some());
    assert!(report.integration_catalog.provider("openrouter").is_none());
}

#[tokio::test]
async fn execute_spec_surfaces_setup_incomplete_plugins_without_marking_them_ready() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_guard = MigrationEnvironmentGuard::set(&[("TAVILY_API_KEY", None)]);

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-setup-incomplete-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let required_env_var = format!("LOONGCLAW_TEST_MISSING_ENV_{unique}");
    let plugin_file = plugin_root.join("tavily_search.py");
    let plugin_manifest = r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "tavily-search",
#   "provider_id": "tavily-search",
#   "connector_name": "tavily-http",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/tavily",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Tavily web search",
#   "setup": {
#     "mode": "metadata_only",
#     "surface": "web_search",
#     "required_env_vars": ["__REQUIRED_ENV_VAR__"],
#     "required_config_keys": ["tools.web_search.default_provider"],
#     "remediation": "configure tavily before enabling search"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#;
    let plugin_manifest = plugin_manifest.replace("__REQUIRED_ENV_VAR__", &required_env_var);
    fs::write(&plugin_file, &plugin_manifest).expect("write plugin file");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-setup-incomplete".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-setup-incomplete".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "tavily".to_owned(),
            limit: 5,
            trust_tiers: Vec::new(),
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;

    if report.operation_kind != "tool_search" {
        panic!(
            "unexpected operation_kind={} blocked_reason={:?} outcome={}",
            report.operation_kind, report.blocked_reason, report.outcome
        );
    }
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].ready_plugins, 0);
    assert_eq!(
        report.plugin_activation_plans[0].setup_incomplete_plugins,
        1
    );
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 0);
    assert!(report.plugin_bootstrap_queue.is_empty());
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(report.outcome["results"][0]["provider_id"], "tavily-search");
    assert_eq!(report.outcome["results"][0]["setup_ready"], false);
    assert_eq!(
        report.outcome["results"][0]["missing_required_env_vars"][0],
        required_env_var
    );
    assert_eq!(
        report.outcome["results"][0]["missing_required_config_keys"][0],
        "tools.web_search.default_provider"
    );
    assert!(
        report
            .integration_catalog
            .provider("tavily-search")
            .is_none()
    );
}

#[tokio::test]
async fn execute_spec_bootstrap_applies_only_bridges_allowed_by_bootstrap_policy() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-bootstrap-selective-{}", unique));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("ffi_plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write ffi plugin");

    fs::write(
        plugin_root.join("http_plugin.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "http-plugin",
//   "provider_id": "http-provider",
//   "connector_name": "http-provider",
//   "channel_id": "primary",
//   "endpoint": "https://hooks.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bootstrap-selective".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bootstrap-selective".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson, PluginBridgeKind::NativeFfi],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(true),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-provider".to_owned(),
            operation: "notify".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].ready_plugins, 2);
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 1);
    assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
    assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
    assert_eq!(report.plugin_bootstrap_queue.len(), 1);
    assert!(
        report
            .integration_catalog
            .provider("http-provider")
            .is_some()
    );
    assert!(
        report
            .integration_catalog
            .provider("ffi-provider")
            .is_none()
    );
}

#[tokio::test]
async fn execute_spec_bootstrap_enforcement_blocks_when_ready_plugins_are_deferred() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-bootstrap-enforce-{}", unique));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("ffi_plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write ffi plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bootstrap-enforce".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bootstrap-enforce".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::NativeFfi],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(true),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bootstrap-enforce".to_owned(),
            objective: "must be blocked by bootstrap enforcement".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason must exist")
            .contains("bootstrap policy blocked")
    );
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 0);
    assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(
        report
            .integration_catalog
            .provider("ffi-provider")
            .is_none()
    );
}

#[tokio::test]
async fn execute_spec_bootstrap_trust_policy_blocks_unverified_high_risk_auto_apply() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-bootstrap-trust-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("ffi_plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write ffi plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bootstrap-trust".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bootstrap-trust".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::NativeFfi],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(true),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: Some(true),
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bootstrap-trust".to_owned(),
            objective: "must be blocked by bootstrap trust policy".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason must exist")
            .contains("bootstrap policy blocked")
    );
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 0);
    assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
    assert_eq!(
        report.plugin_bootstrap_reports[0].tasks[0].trust_tier,
        loongclaw_daemon::kernel::PluginTrustTier::Unverified
    );
    assert_eq!(report.plugin_trust_summary.scanned_plugins, 1);
    assert_eq!(report.plugin_trust_summary.unverified_plugins, 1);
    assert_eq!(report.plugin_trust_summary.high_risk_plugins, 1);
    assert_eq!(report.plugin_trust_summary.high_risk_unverified_plugins, 1);
    assert_eq!(report.plugin_trust_summary.blocked_auto_apply_plugins, 1);
    assert_eq!(report.plugin_trust_summary.review_required_plugins.len(), 1);
    assert_eq!(
        report.plugin_trust_summary.review_required_plugins[0].bridge_kind,
        PluginBridgeKind::NativeFfi
    );
    let audit = report.audit_events.expect("audit events should exist");
    assert!(audit.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::PluginTrustEvaluated {
                scanned_plugins,
                high_risk_unverified_plugins,
                blocked_auto_apply_plugins,
                review_required_plugin_ids,
                review_required_bridges,
                ..
            } if *scanned_plugins == 1
                && *high_risk_unverified_plugins == 1
                && *blocked_auto_apply_plugins == 1
                && review_required_plugin_ids == &vec!["ffi-plugin".to_owned()]
                && review_required_bridges == &vec!["native_ffi".to_owned()]
        )
    }));
    assert!(
        report.plugin_bootstrap_reports[0].tasks[0]
            .reason
            .contains("bootstrap trust policy for unverified high-risk plugins")
    );
    assert!(report.plugin_absorb_reports.is_empty());
}

#[tokio::test]
async fn execute_spec_blocks_on_bridge_support_checksum_mismatch() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bridge-checksum".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bridge-checksum".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: vec!["http-adapter".to_owned()],
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: Some("v1".to_owned()),
            expected_checksum: Some("deadbeef".to_owned()),
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bridge-checksum".to_owned(),
            objective: "should be blocked before execution".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should be present")
            .contains("checksum mismatch")
    );
    assert!(report.bridge_support_checksum.is_some());
}

#[tokio::test]
async fn execute_spec_blocks_on_bridge_support_sha256_mismatch() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bridge-sha256".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bridge-sha256".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: vec!["http-adapter".to_owned()],
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: Some("v2".to_owned()),
            expected_checksum: None,
            expected_sha256: Some("badbad".to_owned()),

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bridge-sha256".to_owned(),
            objective: "should be blocked before execution".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should be present")
            .contains("sha256 mismatch")
    );
    assert!(report.bridge_support_sha256.is_some());
}

#[tokio::test]
async fn execute_spec_allows_execution_when_bridge_support_sha256_matches() {
    let mut bridge_support = BridgeSupportSpec {
        enabled: true,
        supported_bridges: vec![PluginBridgeKind::HttpJson, PluginBridgeKind::ProcessStdio],
        supported_adapter_families: vec!["http-adapter".to_owned()],
        supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
        supported_compatibility_shims: Vec::new(),
        supported_compatibility_shim_profiles: Vec::new(),
        enforce_supported: false,
        policy_version: Some("v2".to_owned()),
        expected_checksum: None,
        expected_sha256: None,
        execute_process_stdio: false,
        execute_http_json: false,
        allowed_process_commands: Vec::new(),
        enforce_execution_success: false,
        security_scan: None,
    };
    bridge_support.expected_sha256 = Some(bridge_support_policy_sha256(&bridge_support));

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bridge-sha256-match".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bridge-sha256-match".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(bridge_support),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bridge-sha256-match".to_owned(),
            objective: "should pass".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert!(report.blocked_reason.is_none());
    assert!(report.bridge_support_sha256.is_some());
}

#[tokio::test]
async fn execute_spec_enriches_plugin_bridge_metadata_and_emits_bridge_execution() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-plugin-bridge-enrich-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("ffi_plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write ffi plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bridge-enrich".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bridge-enrich".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::NativeFfi],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(true),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "ffi-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"demo"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["bridge_kind"],
        "native_ffi"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["entrypoint"],
        "lib::invoke"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["circuit_breaker"]["phase_after"],
        "closed"
    );
    assert_eq!(
        report
            .integration_catalog
            .provider("ffi-provider")
            .expect("provider should exist")
            .metadata
            .get("bridge_kind")
            .cloned(),
        Some("native_ffi".to_owned())
    );
    let runtime_health_json = report
        .integration_catalog
        .provider("ffi-provider")
        .expect("provider should exist")
        .metadata
        .get("plugin_runtime_health_json")
        .cloned()
        .expect("provider metadata should carry runtime health");
    let runtime_health: Value =
        serde_json::from_str(runtime_health_json.as_str()).expect("runtime health should decode");
    assert_eq!(runtime_health["status"], "healthy");
    assert_eq!(runtime_health["circuit_phase"], "closed");
    assert_eq!(runtime_health["consecutive_failures"], 0);
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_executes_when_runtime_enabled() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-wasm-runtime-run-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let digest = Sha256::digest(&wasm_bytes);
    let digest_hex = hex_lower(&digest);

    let plugin_manifest = r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-run",
//   "provider_id": "wasm-runtime-provider",
//   "connector_name": "wasm-runtime-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"__COMPONENT_SHA256__",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#
    .replace("__COMPONENT_SHA256__", digest_hex.as_str());
    fs::write(plugin_root.join("plugin.rs"), plugin_manifest).expect("write wasm plugin manifest");

    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-run".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-run".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: true,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(200_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "executed"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["executor"],
        "wasmtime_module"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["export"],
        "run"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["fuel_limit"],
        200_000
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["fuel_consumed"]
            .is_number()
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_hit"],
        false
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_miss"],
        true
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_inserted"],
        true
    );
    let first_cache_total = report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]
        ["cache_total_module_bytes"]
        .as_u64()
        .expect("cache total bytes should be numeric");
    let first_cache_max =
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_max_bytes"]
            .as_u64()
            .expect("cache max bytes should be numeric");
    assert!(first_cache_total > 0);
    assert!(first_cache_total <= first_cache_max);
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["integrity_check_required"],
        true
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["integrity_check_passed"],
        true
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["expected_sha256"],
        digest_hex
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["artifact_sha256"],
        digest_hex
    );

    let provider = report
        .integration_catalog
        .provider("wasm-runtime-provider")
        .expect("provider should exist");
    let plugin_root_string = plugin_root.display().to_string();
    assert!(provider.metadata.contains_key("plugin_source_path"));
    assert_eq!(
        provider
            .metadata
            .get("plugin_source_kind")
            .map(String::as_str),
        Some("embedded_source")
    );
    assert_eq!(
        provider
            .metadata
            .get("plugin_package_root")
            .map(String::as_str),
        Some(plugin_root_string.as_str())
    );
    assert!(
        !provider
            .metadata
            .contains_key("plugin_package_manifest_path")
    );
    assert!(provider.metadata.contains_key("component_resolved_path"));

    let cached_report = execute_spec(&spec, true).await;
    assert_eq!(
        cached_report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_hit"],
        true
    );
    assert_eq!(
        cached_report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_miss"],
        false
    );
    assert_eq!(
        cached_report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["cache_inserted"],
        false
    );
    assert_eq!(
        cached_report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["integrity_check_required"],
        true
    );
    assert_eq!(
        cached_report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["integrity_check_passed"],
        true
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_when_component_sha256_mismatches() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-wasm-runtime-hash-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let wrong_digest = "00".repeat(32);
    let plugin_manifest = r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-hash",
//   "provider_id": "wasm-runtime-hash-provider",
//   "connector_name": "wasm-runtime-hash-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-hash-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"__WRONG_COMPONENT_SHA256__",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#
    .replace("__WRONG_COMPONENT_SHA256__", wrong_digest.as_str());
    fs::write(plugin_root.join("plugin.rs"), plugin_manifest).expect("write wasm plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-hash".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-hash".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: true,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(200_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-hash-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_sha256_mismatch")
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_when_metadata_pin_conflicts_with_policy_pin() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-pin-conflict-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let digest = Sha256::digest(&wasm_bytes);
    let digest_hex = hex_lower(&digest);
    let wrong_digest = "00".repeat(32);

    let plugin_manifest = r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-pin-conflict",
//   "provider_id": "wasm-runtime-pin-conflict-provider",
//   "connector_name": "wasm-runtime-pin-conflict-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-pin-conflict-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"__COMPONENT_SHA256__",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#
    .replace("__COMPONENT_SHA256__", digest_hex.as_str());
    fs::write(plugin_root.join("plugin.rs"), plugin_manifest).expect("write wasm plugin manifest");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-pin-conflict".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-pin-conflict".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(200_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::from([(
                        "wasm-runtime-pin-conflict".to_owned(),
                        wrong_digest,
                    )]),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-pin-conflict-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "blocked"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("blocked reason should be string")
            .contains("conflicting wasm sha256 pins")
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_when_hash_pin_required_but_missing() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-pin-required-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-pin-required",
//   "provider_id": "wasm-runtime-pin-required-provider",
//   "connector_name": "wasm-runtime-pin-required-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-pin-required-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write wasm plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-pin-required".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-pin-required".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: true,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(200_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: true,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-pin-required-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_sha256_pin_missing"
                && finding.message.contains("hash pin")),
        "expected wasm_sha256_pin_missing finding, got: {:?}",
        security.findings
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_artifact_outside_runtime_prefixes() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-block-path-{unique}"));
    let disallowed_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-deny-prefix-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");
    fs::create_dir_all(&disallowed_root).expect("create disallowed root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-path-block",
//   "provider_id": "wasm-runtime-path-provider",
//   "connector_name": "wasm-runtime-path-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-path-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write wasm plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-block-path".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-block-path".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![disallowed_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(100_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-path-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "blocked"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("blocked reason should be string")
            .contains("outside runtime allowed_path_prefixes")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_symlink_escape_under_allowed_prefix() {
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-symlink-root-{unique}"));
    let outside_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-symlink-outside-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-symlink-block",
//   "provider_id": "wasm-runtime-symlink-provider",
//   "connector_name": "wasm-runtime-symlink-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-symlink-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write wasm plugin manifest");

    let outside_wasm =
        wat::parse_str(r#"(module (func (export "run")))"#).expect("compile outside wasm");
    let outside_wasm_path = outside_root.join("outside.wasm");
    fs::write(&outside_wasm_path, outside_wasm).expect("write outside wasm module");
    symlink(&outside_wasm_path, plugin_root.join("plugin.wasm"))
        .expect("create symlinked wasm artifact");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-symlink-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-symlink-block".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(100_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-symlink-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "blocked"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("blocked reason should be string")
            .contains("outside runtime allowed_path_prefixes")
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_non_regular_artifact_path() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!(
        "loongclaw-wasm-runtime-regular-file-check-{unique}"
    ));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-regular-file-check",
//   "provider_id": "wasm-runtime-regular-file-provider",
//   "connector_name": "wasm-runtime-regular-file-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-regular-file-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write wasm plugin manifest");

    fs::create_dir_all(plugin_root.join("plugin.wasm")).expect("create directory artifact");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-regular-file-check".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-regular-file-check".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(128 * 1024),
                    fuel_limit: Some(100_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-regular-file-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "blocked"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("blocked reason should be string")
            .contains("must reference a regular file")
    );
}

#[tokio::test]
async fn execute_spec_wasm_component_bridge_blocks_when_module_size_exceeds_runtime_limit() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-wasm-runtime-block-size-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-runtime-size-block",
//   "provider_id": "wasm-runtime-size-provider",
//   "connector_name": "wasm-runtime-size-provider",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-runtime-size-provider/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "entrypoint":"run",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write wasm plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let wasm_size = wasm_bytes.len();
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-block-size".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-block-size".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    max_component_bytes: Some(8),
                    fuel_limit: Some(100_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "wasm-runtime-size-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"input":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "blocked"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("blocked reason should be string")
            .contains("exceeds runtime max_component_bytes")
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["module_size_bytes"],
        wasm_size
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["max_component_bytes"],
        8
    );
}

#[tokio::test]
async fn execute_spec_blocks_when_wasm_runtime_enabled_without_allowed_prefixes() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-wasm-runtime-invalid-policy".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-wasm-runtime-invalid-policy".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec {
                    execute_wasm_component: true,
                    allowed_path_prefixes: Vec::new(),
                    max_component_bytes: Some(1024),
                    fuel_limit: Some(10_000),
                    bridge_circuit_breaker: ConnectorCircuitBreakerPolicy::default(),
                },
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec!["examples/plugins-wasm".to_owned()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-wasm-runtime-invalid-policy".to_owned(),
            objective: "runtime policy should fail closed".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("runtime.execute_wasm_component requires runtime.allowed_path_prefixes")
    );
}

#[tokio::test]
async fn execute_spec_security_scan_blocks_wasm_plugin_with_wasi_import() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-security-wasm-block-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-risky",
//   "provider_id": "wasm-risky",
//   "connector_name": "wasm-risky",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-risky/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let wasm_bytes = wat::parse_str(
        r#"(module
                 (import "wasi_snapshot_preview1" "fd_write"
                   (func $fd_write (param i32 i32 i32 i32) (result i32)))
               )"#,
    )
    .expect("compile wasm");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-wasm-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-wasm-block".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-wasm-block".to_owned(),
            objective: "security scan should block risky wasm".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("security scan blocked")
    );
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(security.high_findings > 0);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category.contains("wasi"))
    );
    let audit = report.audit_events.expect("audit events should exist");
    assert!(audit.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::SecurityScanEvaluated {
                blocked,
                high_findings,
                ..
            } if *blocked && *high_findings > 0
        )
    }));
}

#[tokio::test]
async fn execute_spec_security_scan_allows_clean_wasm_with_hash_pin() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-security-wasm-pass-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-clean",
//   "provider_id": "wasm-clean",
//   "connector_name": "wasm-clean",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-clean/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let digest = Sha256::digest(&wasm_bytes);
    let digest_hex = hex_lower(&digest);
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-wasm-pass".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-wasm-pass".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: true,
                    required_sha256_by_plugin: BTreeMap::from([(
                        "wasm-clean".to_owned(),
                        digest_hex.clone(),
                    )]),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-wasm-pass".to_owned(),
            objective: "security scan should allow clean wasm".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(!security.blocked);
    assert_eq!(security.high_findings, 0);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_digest_observed")
    );
    assert!(report.integration_catalog.provider("wasm-clean").is_some());
}

#[tokio::test]
async fn execute_spec_security_scan_allows_clean_wasm_with_metadata_hash_pin() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-security-wasm-pass-metadata-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let digest = Sha256::digest(&wasm_bytes);
    let digest_hex = hex_lower(&digest);

    let plugin_manifest = r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-clean-metadata-pin",
//   "provider_id": "wasm-clean-metadata-pin",
//   "connector_name": "wasm-clean-metadata-pin",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-clean-metadata-pin/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"__COMPONENT_SHA256__",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#
    .replace("__COMPONENT_SHA256__", digest_hex.as_str());
    fs::write(plugin_root.join("plugin.rs"), plugin_manifest).expect("write plugin manifest");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-wasm-pass-metadata".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-wasm-pass-metadata".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: true,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-wasm-pass-metadata".to_owned(),
            objective: "security scan should accept metadata hash pin".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(!security.blocked);
    assert_eq!(security.high_findings, 0);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_digest_observed")
    );
    assert!(
        report
            .integration_catalog
            .provider("wasm-clean-metadata-pin")
            .is_some()
    );
}

#[tokio::test]
async fn execute_spec_security_scan_blocks_when_metadata_hash_pin_is_invalid() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-security-wasm-invalid-pin-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-invalid-metadata-pin",
//   "provider_id": "wasm-invalid-metadata-pin",
//   "connector_name": "wasm-invalid-metadata-pin",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-invalid-metadata-pin/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"sha256:deadbeef",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-wasm-invalid-pin".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-wasm-invalid-pin".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-wasm-invalid-pin".to_owned(),
            objective: "security scan should block invalid metadata hash pin".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(security.high_findings > 0);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_sha256_pin_invalid")
    );
}

#[tokio::test]
async fn execute_spec_security_scan_blocks_when_metadata_pin_conflicts_with_policy_pin() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-security-wasm-pin-conflict-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
    let digest = Sha256::digest(&wasm_bytes);
    let digest_hex = hex_lower(&digest);
    let wrong_digest = "00".repeat(32);

    let plugin_manifest = r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "wasm-conflict",
//   "provider_id": "wasm-conflict",
//   "connector_name": "wasm-conflict",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-conflict/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "component_sha256":"__COMPONENT_SHA256__",
//     "version":"1.0.0"
//   }
// }
// LOONGCLAW_PLUGIN_END
"#
    .replace("__COMPONENT_SHA256__", digest_hex.as_str());
    fs::write(plugin_root.join("plugin.rs"), plugin_manifest).expect("write plugin manifest");
    fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-wasm-pin-conflict".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-wasm-pin-conflict".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::WasmComponent],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: true,
                    max_module_bytes: 128 * 1024,
                    allow_wasi: false,
                    blocked_import_prefixes: vec!["wasi".to_owned()],
                    allowed_path_prefixes: vec![plugin_root.display().to_string()],
                    require_hash_pin: true,
                    required_sha256_by_plugin: BTreeMap::from([(
                        "wasm-conflict".to_owned(),
                        wrong_digest,
                    )]),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(true),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-wasm-pin-conflict".to_owned(),
            objective: "security scan should block conflicting wasm hash pins".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(security.high_findings > 0);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_sha256_pin_conflict")
    );
}

#[tokio::test]
async fn execute_spec_security_scan_emits_audit_summary_when_not_blocking() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-security-audit-pass-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "stdio-audit",
#   "provider_id": "stdio-audit",
#   "connector_name": "stdio-audit",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-audit/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-audit-pass".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-audit-pass".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(true),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-audit-pass".to_owned(),
            objective: "security scan should emit audit summary".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(!security.blocked);
    assert!(security.high_findings >= 1);

    let audit = report.audit_events.expect("audit events should exist");
    #[allow(clippy::wildcard_enum_match_arm)]
    let summary = audit.iter().find_map(|event| match &event.kind {
        AuditEventKind::SecurityScanEvaluated {
            blocked,
            high_findings,
            categories,
            finding_ids,
            ..
        } => Some((
            *blocked,
            *high_findings,
            categories.clone(),
            finding_ids.clone(),
        )),
        _ => None,
    });

    let (blocked, high_findings, categories, finding_ids) =
        summary.expect("security scan audit summary should exist");
    assert!(!blocked);
    assert!(high_findings >= 1);
    assert!(
        categories
            .iter()
            .any(|value| value == "process_command_not_allowlisted")
    );
    assert!(!finding_ids.is_empty());
    assert!(finding_ids.iter().all(|value| value.starts_with("sf-")));
}

#[tokio::test]
async fn execute_spec_security_scan_exports_siem_record_with_truncation() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-security-siem-pass-{unique}"));
    let siem_path =
        std::env::temp_dir().join(format!("loongclaw-security-siem-pass-{unique}.jsonl"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "stdio-siem",
#   "provider_id": "stdio-siem",
#   "connector_name": "stdio-siem",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-siem/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "note":"shell-enabled",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-siem-pass".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-siem-pass".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: Some(SecuritySiemExportSpec {
                    enabled: true,
                    path: siem_path.display().to_string(),
                    include_findings: true,
                    max_findings_per_record: Some(1),
                    fail_on_error: true,
                }),
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: vec!["shell".to_owned()],
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(true),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-siem-pass".to_owned(),
            objective: "security scan should export siem record".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    let siem = security
        .siem_export
        .expect("siem export report should exist");
    assert!(siem.success);
    assert_eq!(siem.exported_records, 1);
    assert_eq!(siem.exported_findings, 1);
    assert!(siem.truncated_findings >= 1);

    let siem_body = fs::read_to_string(&siem_path).expect("read siem record");
    let first_line = siem_body.lines().next().expect("one siem line");
    let record: Value = serde_json::from_str(first_line).expect("parse siem json");
    assert_eq!(record["event_type"], "security_scan_report");
    assert_eq!(record["pack_id"], "spec-security-siem-pass");
    assert_eq!(record["agent_id"], "agent-security-siem-pass");
    assert!(record["findings"].as_array().map_or(0, Vec::len) == 1);
    assert!(record["truncated_findings"].as_u64().unwrap_or_default() >= 1);
    assert!(record["finding_ids"].as_array().map_or(0, Vec::len) >= 2);
}

#[tokio::test]
async fn execute_spec_security_scan_siem_fail_closed_blocks_execution() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-security-siem-block-{unique}"));
    let invalid_parent =
        std::env::temp_dir().join(format!("loongclaw-security-siem-parent-file-{unique}.tmp"));
    let invalid_siem_path = invalid_parent.join("events.jsonl");
    fs::create_dir_all(&plugin_root).expect("create plugin root");
    fs::write(&invalid_parent, "not-a-directory").expect("create invalid parent marker file");

    fs::write(
        plugin_root.join("plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "stdio-siem-block",
#   "provider_id": "stdio-siem-block",
#   "connector_name": "stdio-siem-block",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-siem-block/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-siem-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-siem-block".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: false,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: Some(SecuritySiemExportSpec {
                    enabled: true,
                    path: invalid_siem_path.display().to_string(),
                    include_findings: true,
                    max_findings_per_record: None,
                    fail_on_error: true,
                }),
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(true),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-siem-block".to_owned(),
            objective: "siem fail closed should block".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("siem export failed")
    );
    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    let siem = security
        .siem_export
        .expect("siem export report should exist");
    assert!(!siem.success);
    assert!(siem.error.is_some());
}

#[tokio::test]
async fn execute_spec_security_scan_covers_deferred_plugins_not_only_applied_subset() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-security-deferred-ready-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("01-safe.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "stdio-safe",
#   "provider_id": "stdio-safe",
#   "connector_name": "stdio-safe",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-safe/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write safe plugin");

    fs::write(
        plugin_root.join("02-risky.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "stdio-risky",
#   "provider_id": "stdio-risky",
#   "connector_name": "stdio-risky",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-risky/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write risky plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-security-deferred-ready".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-security-deferred-ready".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
            enforce_execution_success: false,
            security_scan: Some(SecurityScanSpec {
                enabled: true,
                block_on_high: true,
                profile_path: None,
                profile_sha256: None,
                profile_signature: None,
                siem_export: None,
                runtime: SecurityRuntimeExecutionSpec::default(),
                high_risk_metadata_keywords: Vec::new(),
                wasm: WasmSecurityScanSpec {
                    enabled: false,
                    max_module_bytes: 0,
                    allow_wasi: false,
                    blocked_import_prefixes: Vec::new(),
                    allowed_path_prefixes: Vec::new(),
                    require_hash_pin: false,
                    required_sha256_by_plugin: BTreeMap::new(),
                },
            }),
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(true),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-security-deferred-ready".to_owned(),
            objective: "security scan must inspect deferred ready plugins".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("security scan blocked")
    );
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert!(report.plugin_bootstrap_reports[0].total_tasks >= 1);
    assert_eq!(report.plugin_scan_reports[0].matched_plugins, 2);

    let security = report
        .security_scan_report
        .expect("security scan report should exist");
    assert!(security.blocked);
    assert!(security.high_findings >= 1);
    assert!(
        security
            .findings
            .iter()
            .any(|finding| finding.plugin_id == "stdio-risky")
    );
    assert!(report.plugin_absorb_reports.is_empty());
}

#[tokio::test]
async fn execute_spec_default_medium_policy_blocks_high_risk_tool_call_without_approval() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-default-block".to_owned(),
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
        agent_id: "agent-approval-default".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert_eq!(report.outcome["status"], "blocked");
    assert!(report.approval_guard.requires_human_approval);
    assert!(!report.approval_guard.approved);
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("human approval required")
    );
}

#[tokio::test]
async fn execute_spec_per_call_approval_allows_high_risk_tool_call() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-per-call".to_owned(),
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
        agent_id: "agent-approval-per-call".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::MediumBalanced,
            strategy: HumanApprovalStrategy::PerCall,
            approved_calls: vec!["tool_core:delete-file".to_owned()],
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_core");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert!(report.approval_guard.requires_human_approval);
    assert!(report.approval_guard.approved);
}

#[tokio::test]
async fn execute_spec_one_time_full_access_allows_high_risk_tool_call() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-once-full".to_owned(),
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
        agent_id: "agent-approval-once-full".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::MediumBalanced,
            strategy: HumanApprovalStrategy::OneTimeFullAccess,
            one_time_full_access_granted: true,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_core");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert!(report.approval_guard.requires_human_approval);
    assert!(report.approval_guard.approved);
}

#[tokio::test]
async fn execute_spec_strict_mode_requires_approval_for_low_risk_tool_call() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-strict".to_owned(),
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
        agent_id: "agent-approval-strict".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Strict,
            strategy: HumanApprovalStrategy::PerCall,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "read-schema".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"scope":"analytics"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(report.approval_guard.requires_human_approval);
    assert!(!report.approval_guard.approved);
}

#[tokio::test]
async fn execute_spec_default_medium_policy_allows_low_risk_tool_call_without_approval() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-default-allow".to_owned(),
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
        agent_id: "agent-approval-default-allow".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "list-schema".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"scope":"analytics"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_core");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert!(!report.approval_guard.requires_human_approval);
    assert!(report.approval_guard.approved);
    assert_eq!(report.approval_guard.risk_level, ApprovalRiskLevel::Low);
}

#[tokio::test]
async fn execute_spec_tool_core_can_run_claw_migrate_plan_via_native_tool_runtime() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-spec-tool-core-migrate");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(
        &root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );
    write_file(
        &root,
        "IDENTITY.md",
        "# Identity\n\n- Motto: your nanobot agent for deploys\n",
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-core-claw-migrate".to_owned(),
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
        agent_id: "agent-tool-core-claw-migrate".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "claw.migrate".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "mode": "plan",
                "source": "nanobot",
                "input_path": root.display().to_string()
            }),
            core: None,
        },
    };

    let report =
        execute_spec_with_native_tool_executor(&spec, true, Some(native_spec_tool_executor)).await;
    assert_eq!(report.operation_kind, "tool_core");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.outcome["outcome"]["payload"]["source"], "nanobot");
    assert_eq!(
        report.outcome["outcome"]["payload"]["config_preview"]["prompt_pack_id"],
        "loongclaw-core-v1"
    );
    assert!(
        report.outcome["outcome"]["payload"]["config_preview"]["system_prompt_addendum"]
            .as_str()
            .expect("prompt addendum should exist")
            .contains("LoongClaw")
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_spec_tool_extension_can_hot_handle_claw_migrate_via_core_wrapper() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-spec-tool-extension-import");
    fs::create_dir_all(&root).expect("create fixture root");
    write_file(
        &root,
        "SOUL.md",
        "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-extension-claw-import".to_owned(),
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
        agent_id: "agent-tool-extension-claw-import".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolExtension {
            extension_action: "plan".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "source": "nanobot",
                "input_path": root.display().to_string()
            }),
            extension: "claw-migration".to_owned(),
            core: None,
        },
    };

    let report =
        execute_spec_with_native_tool_executor(&spec, true, Some(native_spec_tool_executor)).await;
    assert_eq!(report.operation_kind, "tool_extension");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["extension"],
        "claw-migration"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["core_outcome"]["mode"],
        "plan"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["core_outcome"]["source"],
        "nanobot"
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_spec_tool_extension_can_discover_multiple_sources() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-spec-tool-extension-discover-many");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n",
    );

    let nanobot_root = root.join("nanobot");
    fs::create_dir_all(&nanobot_root).expect("create nanobot root");
    write_file(
        &nanobot_root,
        "IDENTITY.md",
        "# Identity\n\n- Motto: your nanobot agent for deploys\n",
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-extension-claw-discover-many".to_owned(),
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
        agent_id: "agent-tool-extension-claw-discover-many".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolExtension {
            extension_action: "discover".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "input_path": root.display().to_string()
            }),
            extension: "claw-migration".to_owned(),
            core: None,
        },
    };

    let report =
        execute_spec_with_native_tool_executor(&spec, true, Some(native_spec_tool_executor)).await;
    assert_eq!(report.operation_kind, "tool_extension");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.outcome["outcome"]["payload"]["action"], "discover");
    assert!(
        report.outcome["outcome"]["payload"]["sources"]
            .as_array()
            .expect("sources should be an array")
            .len()
            >= 2
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_spec_tool_extension_can_merge_profiles_without_merging_prompt_lane() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-spec-tool-extension-merge-profiles");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- tone: steady\n",
    );

    let nanobot_root = root.join("nanobot");
    fs::create_dir_all(&nanobot_root).expect("create nanobot root");
    write_file(
        &nanobot_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n- region: apac\n",
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-extension-claw-merge-profiles".to_owned(),
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
        agent_id: "agent-tool-extension-claw-merge-profiles".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolExtension {
            extension_action: "merge_profiles".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "input_path": root.display().to_string()
            }),
            extension: "claw-migration".to_owned(),
            core: None,
        },
    };

    let report =
        execute_spec_with_native_tool_executor(&spec, true, Some(native_spec_tool_executor)).await;
    assert_eq!(report.operation_kind, "tool_extension");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["action"],
        "merge_profiles"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["result"]["prompt_owner_source_id"],
        "openclaw"
    );

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_spec_tool_extension_apply_selected_safe_merge_keeps_native_prompt() {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    fn write_file(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write fixture");
    }

    let root = unique_temp_dir("loongclaw-spec-tool-extension-apply-safe-merge");
    fs::create_dir_all(&root).expect("create fixture root");

    let openclaw_root = root.join("openclaw-workspace");
    fs::create_dir_all(&openclaw_root).expect("create openclaw root");
    write_file(
        &openclaw_root,
        "SOUL.md",
        "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
    );
    write_file(
        &openclaw_root,
        "IDENTITY.md",
        "# Identity\n\n- role: release copilot\n",
    );

    let nanobot_root = root.join("nanobot");
    fs::create_dir_all(&nanobot_root).expect("create nanobot root");
    write_file(
        &nanobot_root,
        "IDENTITY.md",
        "# Identity\n\n- region: apac\n",
    );

    let output_path = root.join("loongclaw.toml");
    let mut existing = loongclaw_app::config::LoongClawConfig::default();
    existing.cli.system_prompt_addendum = Some("Native LoongClaw prompt".to_owned());
    let existing_body = loongclaw_app::config::render(&existing).expect("render existing config");
    fs::write(&output_path, existing_body).expect("write existing config");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-extension-claw-apply-safe-merge".to_owned(),
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
        agent_id: "agent-tool-extension-claw-apply-safe-merge".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolExtension {
            extension_action: "apply_selected".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "input_path": root.display().to_string(),
                "output_path": output_path.display().to_string(),
                "safe_profile_merge": true,
                "primary_source_id": "openclaw"
            }),
            extension: "claw-migration".to_owned(),
            core: None,
        },
    };

    let report =
        execute_spec_with_native_tool_executor(&spec, true, Some(native_spec_tool_executor)).await;
    assert_eq!(report.operation_kind, "tool_extension");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["action"],
        "apply_selected"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["result"]["prompt_owner_source_id"],
        serde_json::Value::Null
    );

    let output_string = output_path.display().to_string();
    let (_, merged_config) =
        loongclaw_app::config::load(Some(&output_string)).expect("load merged config");
    assert_eq!(
        merged_config.cli.system_prompt_addendum.as_deref(),
        Some("Native LoongClaw prompt")
    );
    let profile_note = merged_config
        .memory
        .profile_note
        .as_deref()
        .expect("profile note should be present");
    assert!(profile_note.contains("role: release copilot"));
    assert!(profile_note.contains("region: apac"));

    fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn execute_spec_denylist_overrides_other_approvals() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-denylist".to_owned(),
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
        agent_id: "agent-approval-denylist".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            strategy: HumanApprovalStrategy::PerCall,
            approved_calls: vec!["tool_core:delete-file".to_owned()],
            denied_calls: vec!["tool_core:delete-file".to_owned()],
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(report.approval_guard.denylisted);
    assert!(!report.approval_guard.approved);
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("denylisted")
    );
}

#[tokio::test]
async fn execute_spec_one_time_full_access_expired_is_rejected() {
    let now = current_epoch_s();
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-full-expired".to_owned(),
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
        agent_id: "agent-approval-full-expired".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Strict,
            strategy: HumanApprovalStrategy::OneTimeFullAccess,
            one_time_full_access_granted: true,
            one_time_full_access_expires_at_epoch_s: Some(now.saturating_sub(1)),
            one_time_full_access_remaining_uses: Some(1),
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(!report.approval_guard.approved);
    assert!(report.approval_guard.reason.contains("expired"));
}

#[tokio::test]
async fn execute_spec_one_time_full_access_with_zero_remaining_uses_is_rejected() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-approval-full-zero-uses".to_owned(),
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
        agent_id: "agent-approval-full-zero-uses".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Strict,
            strategy: HumanApprovalStrategy::OneTimeFullAccess,
            one_time_full_access_granted: true,
            one_time_full_access_remaining_uses: Some(0),
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolCore {
            tool_name: "delete-file".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({"path":"/tmp/demo.txt"}),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(!report.approval_guard.approved);
    assert!(report.approval_guard.reason.contains("no remaining uses"));
}

#[tokio::test]
async fn execute_spec_bootstrap_max_tasks_limits_applied_plugins() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-bootstrap-limit-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_a.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "http-a",
//   "provider_id": "http-a",
//   "connector_name": "http-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write http plugin a");

    fs::write(
        plugin_root.join("http_b.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "http-b",
//   "provider_id": "http-b",
//   "connector_name": "http-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write http plugin b");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bootstrap-limit".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bootstrap-limit".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(true),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bootstrap-limit".to_owned(),
            objective: "run regardless of selective bootstrap".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(report.plugin_bootstrap_reports.len(), 1);
    assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 1);
    assert_eq!(report.plugin_bootstrap_reports[0].skipped_tasks, 1);
    assert_eq!(report.plugin_absorb_reports.len(), 1);
    assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
}

#[tokio::test]
async fn execute_spec_scans_multiple_roots_and_absorbs_per_root() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let root_a = std::env::temp_dir().join(format!("loongclaw-plugin-root-a-{unique}"));
    let root_b = std::env::temp_dir().join(format!("loongclaw-plugin-root-b-{unique}"));
    fs::create_dir_all(&root_a).expect("create root a");
    fs::create_dir_all(&root_b).expect("create root b");

    fs::write(
        root_a.join("a.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "root-a",
//   "provider_id": "root-a",
//   "connector_name": "root-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root a plugin");

    fs::write(
        root_b.join("b.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "root-b",
//   "provider_id": "root-b",
//   "connector_name": "root-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-multi-root".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-multi-root".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![root_a.display().to_string(), root_b.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(true),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-multi-root".to_owned(),
            objective: "validate multi-root scan".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.plugin_scan_reports.len(), 2);
    assert_eq!(report.plugin_absorb_reports.len(), 2);
    let absorbed_total: usize = report
        .plugin_absorb_reports
        .iter()
        .map(|entry| entry.absorbed_plugins)
        .sum();
    assert_eq!(absorbed_total, 2);
    assert!(report.integration_catalog.provider("root-a").is_some());
    assert!(report.integration_catalog.provider("root-b").is_some());
}

#[tokio::test]
async fn execute_spec_blocks_cross_root_slot_claim_conflicts_during_planning() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let root_a = std::env::temp_dir().join(format!("loongclaw-plugin-slot-a-{unique}"));
    let root_b = std::env::temp_dir().join(format!("loongclaw-plugin-slot-b-{unique}"));
    fs::create_dir_all(&root_a).expect("create root a");
    fs::create_dir_all(&root_b).expect("create root b");

    fs::write(
        root_a.join("a.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-a",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root a plugin");

    fs::write(
        root_b.join("b.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-b",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-cross-root-slot-claims".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-cross-root-slot-claims".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![root_a.display().to_string(), root_b.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-cross-root-slot-claims".to_owned(),
            objective: "detect cross-root slot conflicts during planning".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    let blocked_reason = report.blocked_reason.as_deref().unwrap_or_default();
    let second_plan = &report.plugin_activation_plans[1];
    let second_candidate = &second_plan.candidates[0];

    assert_eq!(report.operation_kind, "blocked");
    assert!(blocked_reason.contains("blocked_slot_claim_conflict"));
    assert_eq!(report.plugin_activation_plans.len(), 2);
    assert_eq!(second_plan.blocked_plugins, 1);
    assert_eq!(
        second_candidate.status,
        PluginActivationStatus::BlockedSlotClaimConflict
    );
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(report.integration_catalog.provider("search-a").is_none());
    assert!(report.integration_catalog.provider("search-b").is_none());
}

#[tokio::test]
async fn execute_spec_plugin_scan_is_transactional_when_blocked() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let root_a = std::env::temp_dir().join(format!("loongclaw-plugin-rollback-a-{unique}"));
    let root_b = std::env::temp_dir().join(format!("loongclaw-plugin-rollback-b-{unique}"));
    fs::create_dir_all(&root_a).expect("create root a");
    fs::create_dir_all(&root_b).expect("create root b");

    fs::write(
        root_a.join("a.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "rollback-a",
//   "provider_id": "rollback-a",
//   "connector_name": "rollback-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root a plugin");

    fs::write(
        root_b.join("b.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "rollback-b",
//   "provider_id": "rollback-b",
//   "connector_name": "rollback-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-rollback".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-rollback".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![root_a.display().to_string(), root_b.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-plugin-rollback".to_owned(),
            objective: "must block and rollback staged plugin absorb".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    let blocked_reason = report.blocked_reason.expect("blocked reason");
    assert!(blocked_reason.contains("bridge support enforcement blocked"));
    assert!(blocked_reason.contains("rollback-b"));
    assert!(blocked_reason.contains("blocked_unsupported_bridge"));
    assert_eq!(report.plugin_scan_reports.len(), 2);
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(report.integration_catalog.provider("rollback-a").is_none());
    assert!(report.integration_catalog.provider("rollback-b").is_none());
}

#[tokio::test]
async fn execute_spec_blocks_when_package_manifest_conflicts_with_source_manifest() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let plugin_root = std::env::temp_dir().join(format!("loongclaw-plugin-conflict-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("loongclaw.plugin.json"),
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "conflict-plugin",
  "provider_id": "package-provider",
  "connector_name": "conflict-connector",
  "channel_id": "primary",
  "endpoint": "https://package.example.com/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
    )
    .expect("write package manifest");

    fs::write(
        plugin_root.join("plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "conflict-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "conflict-connector",
#   "channel_id": "primary",
#   "endpoint": "https://package.example.com/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write conflicting source manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-manifest-conflict".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-manifest-conflict".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-plugin-manifest-conflict".to_owned(),
            objective: "plugin scan should fail on package/source drift".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;

    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("plugin manifest conflict")
    );
    assert!(report.plugin_scan_reports.is_empty());
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(
        report
            .integration_catalog
            .provider("package-provider")
            .is_none()
    );
    assert!(
        report
            .integration_catalog
            .provider("source-provider")
            .is_none()
    );
}

#[tokio::test]
async fn execute_spec_blocks_when_package_manifest_uses_legacy_version_metadata() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-legacy-version-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("loongclaw.plugin.json"),
        r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "legacy-version-package",
  "provider_id": "legacy-version-package",
  "connector_name": "legacy-version-package",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "version": "1.0.0"
  }
}
"#,
    )
    .expect("write package manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-package-legacy-version".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-package-legacy-version".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-plugin-package-legacy-version".to_owned(),
            objective: "package manifest should reject legacy metadata.version".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;

    assert_eq!(report.operation_kind, "blocked");
    let blocked_reason = report.blocked_reason.expect("blocked reason should exist");
    assert!(blocked_reason.contains("metadata.version"));
    assert!(blocked_reason.contains("top-level `version`"));
    assert!(report.plugin_scan_reports.is_empty());
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(
        report
            .integration_catalog
            .provider("legacy-version-package")
            .is_none()
    );
}

#[tokio::test]
async fn execute_spec_bootstrap_budget_is_global_across_multiple_roots() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();

    let root_a = std::env::temp_dir().join(format!("loongclaw-bootstrap-global-a-{unique}"));
    let root_b = std::env::temp_dir().join(format!("loongclaw-bootstrap-global-b-{unique}"));
    fs::create_dir_all(&root_a).expect("create root a");
    fs::create_dir_all(&root_b).expect("create root b");

    fs::write(
        root_a.join("a.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "global-a",
//   "provider_id": "global-a",
//   "connector_name": "global-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root a plugin");

    fs::write(
        root_b.join("b.js"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "global-b",
//   "provider_id": "global-b",
//   "connector_name": "global-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write root b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-bootstrap-global".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-bootstrap-global".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![root_a.display().to_string(), root_b.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,

            execute_process_stdio: false,

            execute_http_json: false,

            allowed_process_commands: Vec::new(),

            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(true),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::Task {
            task_id: "t-bootstrap-global".to_owned(),
            objective: "max_tasks must be global across roots".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "task");
    assert_eq!(report.plugin_bootstrap_reports.len(), 2);
    let total_applied: usize = report
        .plugin_bootstrap_reports
        .iter()
        .map(|entry| entry.applied_tasks)
        .sum();
    let total_skipped: usize = report
        .plugin_bootstrap_reports
        .iter()
        .map(|entry| entry.skipped_tasks)
        .sum();
    assert_eq!(total_applied, 1);
    assert_eq!(total_skipped, 1);

    let total_absorbed: usize = report
        .plugin_absorb_reports
        .iter()
        .map(|entry| entry.absorbed_plugins)
        .sum();
    assert_eq!(total_absorbed, 1);
    assert!(report.integration_catalog.provider("global-a").is_some());
    assert!(report.integration_catalog.provider("global-b").is_none());
}

#[tokio::test]
async fn execute_spec_tool_search_honors_deferred_filter_and_examples() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-tool-search-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("openrouter_research.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "openrouter-research",
#   "provider_id": "openrouter-research",
#   "connector_name": "openrouter-research",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/openrouter",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Deep web search and synthesis",
#   "tags": ["search", "research", "web"],
#   "input_examples": [{"query":"search best rust crates"}],
#   "output_examples": [{"answer":"top crates"}],
#   "defer_loading": true
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin a");

    fs::write(
        plugin_root.join("search_docs.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "doc-search",
#   "provider_id": "doc-search",
#   "connector_name": "doc-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/docs",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search docs",
#   "tags": ["search"],
#   "input_examples": [{"query":"search docs"}],
#   "output_examples": [{"answer":"docs"}],
#   "defer_loading": true
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write plugin b");

    let base_spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "web search".to_owned(),
            limit: 5,
            trust_tiers: Vec::new(),
            include_deferred: false,
            include_examples: false,
        },
    };

    let report_hidden_deferred = execute_spec(&base_spec, true).await;
    assert_eq!(
        report_hidden_deferred.operation_kind, "tool_search",
        "blocked_reason={:?}, outcome={}",
        report_hidden_deferred.blocked_reason, report_hidden_deferred.outcome
    );
    assert_eq!(report_hidden_deferred.outcome["returned"], 0);

    let mut visible_spec = base_spec;
    visible_spec.operation = OperationSpec::ToolSearch {
        query: "web search".to_owned(),
        limit: 5,
        trust_tiers: Vec::new(),
        include_deferred: true,
        include_examples: true,
    };

    let report_visible_deferred = execute_spec(&visible_spec, true).await;
    assert_eq!(
        report_visible_deferred.operation_kind, "tool_search",
        "blocked_reason={:?}, outcome={}",
        report_visible_deferred.blocked_reason, report_visible_deferred.outcome
    );
    assert_eq!(report_visible_deferred.outcome["returned"], 2);
    assert_eq!(
        report_visible_deferred.outcome["results"][0]["provider_id"],
        "openrouter-research"
    );
    assert_eq!(
        report_visible_deferred.outcome["results"][0]["input_examples"][0]["query"],
        "search best rust crates"
    );
}

#[tokio::test]
async fn execute_spec_tool_search_uses_explicit_plugin_setup_readiness_context() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-readiness-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    let plugin_manifest_path = plugin_root.join("loongclaw.plugin.json");
    fs::write(
        &plugin_manifest_path,
        r#"
{
  "api_version": "v1alpha1",
  "plugin_id": "tavily-search",
  "provider_id": "tavily",
  "connector_name": "tavily-http",
  "version": "1.0.0",
  "endpoint": "https://api.tavily.com/search",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search"
  },
  "summary": "Manifest-discovered Tavily package",
  "tags": ["search", "provider"],
  "setup": {
    "mode": "metadata_only",
    "surface": "web_search",
    "required_env_vars": ["TAVILY_API_KEY"],
    "required_config_keys": ["tools.web_search.default_provider"],
    "default_env_var": "TAVILY_API_KEY",
    "docs_urls": ["https://docs.example.com/tavily"],
    "remediation": "set a Tavily credential before enabling search"
  }
}
"#,
    )
    .expect("write plugin manifest");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-readiness".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-readiness".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: Some(PluginSetupReadinessSpec {
            inherit_process_env: false,
            verified_env_vars: vec!["TAVILY_API_KEY".to_owned()],
            verified_config_keys: vec!["tools.web_search.default_provider".to_owned()],
        }),
        operation: OperationSpec::ToolSearch {
            query: "tavily".to_owned(),
            limit: 5,
            trust_tiers: Vec::new(),
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;

    if report.operation_kind != "tool_search" {
        panic!(
            "unexpected operation_kind={} blocked_reason={:?} outcome={}",
            report.operation_kind, report.blocked_reason, report.outcome
        );
    }
    assert_eq!(report.plugin_activation_plans.len(), 1);
    assert_eq!(report.plugin_activation_plans[0].ready_plugins, 1);
    assert_eq!(
        report.plugin_activation_plans[0].setup_incomplete_plugins,
        0
    );
    assert!(matches!(
        report.plugin_activation_plans[0].candidates[0].status,
        loongclaw_daemon::kernel::PluginActivationStatus::Ready
    ));
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(report.outcome["results"][0]["provider_id"], "tavily");
    assert_eq!(report.outcome["results"][0]["setup_ready"], true);
    assert_eq!(
        report.outcome["results"][0]["missing_required_env_vars"],
        json!([])
    );
    assert_eq!(
        report.outcome["results"][0]["missing_required_config_keys"],
        json!([])
    );
}

#[tokio::test]
async fn execute_spec_tool_search_uses_translation_bridge_kind_for_unabsorbed_plugins() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-translation-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("rusty_plugin.rs"),
        r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "rusty-search",
//   "provider_id": "rusty-search",
//   "connector_name": "rusty-search",
//   "channel_id": "primary",
//   "endpoint": "https://example.com/rusty",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"1.0.0"},
//   "summary": "Rust-native search plugin"
// }
// LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write translation plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-translation".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-translation".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::NativeFfi],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "rusty".to_owned(),
            limit: 5,
            trust_tiers: Vec::new(),
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_search");
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(report.outcome["results"][0]["provider_id"], "rusty-search");
    assert_eq!(report.outcome["results"][0]["bridge_kind"], "native_ffi");
    assert_eq!(
        report.outcome["results"][0]["adapter_family"],
        "rust-ffi-adapter"
    );
}

#[tokio::test]
async fn execute_spec_tool_search_filters_by_trust_tier_query_prefix() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-trust-filter-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("official_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "official-search",
#   "provider_id": "official-search",
#   "connector_name": "official-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/official-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search trusted official docs",
#   "trust_tier": "official"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write official plugin");

    fs::write(
        plugin_root.join("unverified_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "unverified-search",
#   "provider_id": "unverified-search",
#   "connector_name": "unverified-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/unverified-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search unreviewed docs",
#   "trust_tier": "unverified"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write unverified plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-trust-filter".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-trust-filter".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "trust:official search".to_owned(),
            limit: 5,
            trust_tiers: Vec::new(),
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_search");
    let search_summary = report
        .tool_search_summary
        .as_ref()
        .expect("tool_search report should expose top-level summary");
    assert_eq!(
        search_summary.headline,
        "query=\"trust:official search\"; returned 1 result; trust_scope=official; filtered_out=1 candidate; top_match=official-search"
    );
    assert_eq!(search_summary.query, "trust:official search");
    assert_eq!(search_summary.returned, 1);
    assert_eq!(
        search_summary.trust_filter_summary.query_requested_tiers,
        vec!["official".to_owned()]
    );
    assert!(
        !search_summary
            .trust_filter_summary
            .conflicting_requested_tiers
    );
    assert_eq!(report.outcome["trust_filter_summary"]["applied"], true);
    assert_eq!(
        report.outcome["trust_filter_summary"]["query_requested_tiers"],
        json!(["official"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["effective_tiers"],
        json!(["official"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["conflicting_requested_tiers"],
        false
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_candidates"],
        1
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_tier_counts"]["unverified"],
        1
    );
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(
        report.outcome["results"][0]["provider_id"],
        "official-search"
    );
    assert_eq!(report.outcome["results"][0]["trust_tier"], "official");
}

#[tokio::test]
async fn execute_spec_tool_search_filters_by_structured_trust_tiers() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-structured-trust-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("official_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "official-search",
#   "provider_id": "official-search",
#   "connector_name": "official-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/official-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search trusted official docs",
#   "trust_tier": "official"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write official plugin");

    fs::write(
        plugin_root.join("verified_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "verified-search",
#   "provider_id": "verified-search",
#   "connector_name": "verified-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/verified-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search trusted community docs",
#   "trust_tier": "verified-community"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write verified plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-structured-trust".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-structured-trust".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "search".to_owned(),
            limit: 5,
            trust_tiers: vec![loongclaw_daemon::kernel::PluginTrustTier::Official],
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_search");
    let search_summary = report
        .tool_search_summary
        .as_ref()
        .expect("tool_search report should expose top-level summary");
    assert_eq!(
        search_summary.headline,
        "query=\"search\"; returned 1 result; trust_scope=official; filtered_out=1 candidate; top_match=official-search"
    );
    assert_eq!(search_summary.query, "search");
    assert_eq!(search_summary.returned, 1);
    assert_eq!(search_summary.trust_tiers, vec!["official".to_owned()]);
    assert!(
        !search_summary
            .trust_filter_summary
            .conflicting_requested_tiers
    );
    assert_eq!(report.outcome["trust_tiers"], json!(["official"]));
    assert_eq!(report.outcome["trust_filter_summary"]["applied"], true);
    assert_eq!(
        report.outcome["trust_filter_summary"]["structured_requested_tiers"],
        json!(["official"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["effective_tiers"],
        json!(["official"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["conflicting_requested_tiers"],
        false
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_candidates"],
        1
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_tier_counts"]["verified-community"],
        1
    );
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(
        report.outcome["results"][0]["provider_id"],
        "official-search"
    );
    assert_eq!(report.outcome["results"][0]["trust_tier"], "official");
}

#[tokio::test]
async fn execute_spec_tool_search_conflicting_trust_filters_fail_closed() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-conflicting-trust-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("official_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "official-search",
#   "provider_id": "official-search",
#   "connector_name": "official-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/official-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search trusted official docs",
#   "trust_tier": "official"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write official plugin");

    fs::write(
        plugin_root.join("verified_search.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "verified-search",
#   "provider_id": "verified-search",
#   "connector_name": "verified-search",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/verified-search",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "summary": "Search trusted community docs",
#   "trust_tier": "verified-community"
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write verified plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-conflicting-trust".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-conflicting-trust".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![PluginCompatibilityMode::Native],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: Vec::new(),
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(false),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "trust:official search".to_owned(),
            limit: 5,
            trust_tiers: vec![loongclaw_daemon::kernel::PluginTrustTier::VerifiedCommunity],
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_search");
    let search_summary = report
        .tool_search_summary
        .as_ref()
        .expect("tool_search report should expose top-level summary");
    assert_eq!(
        search_summary.headline,
        "query=\"trust:official search\"; returned 0 results; trust_scope=none; filtered_out=2 candidates; conflicting_trust_filters=true"
    );
    assert_eq!(search_summary.query, "trust:official search");
    assert_eq!(search_summary.returned, 0);
    assert!(search_summary.top_results.is_empty());
    assert_eq!(
        search_summary.trust_tiers,
        vec!["verified-community".to_owned()]
    );
    assert!(
        search_summary
            .trust_filter_summary
            .effective_tiers
            .is_empty()
    );
    assert!(
        search_summary
            .trust_filter_summary
            .conflicting_requested_tiers
    );
    assert_eq!(report.outcome["trust_tiers"], json!(["verified-community"]));
    assert_eq!(report.outcome["trust_filter_summary"]["applied"], true);
    assert_eq!(
        report.outcome["trust_filter_summary"]["query_requested_tiers"],
        json!(["official"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["structured_requested_tiers"],
        json!(["verified-community"])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["effective_tiers"],
        json!([])
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["conflicting_requested_tiers"],
        true
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_candidates"],
        2
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_tier_counts"]["official"],
        1
    );
    assert_eq!(
        report.outcome["trust_filter_summary"]["filtered_out_tier_counts"]["verified-community"],
        1
    );
    assert_eq!(report.outcome["returned"], 0);
    assert_eq!(report.outcome["results"], json!([]));
    let audit = report.audit_events.expect("audit events should exist");
    assert!(audit.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolSearchEvaluated {
                pack_id,
                query,
                returned,
                trust_filter_applied,
                query_requested_tiers,
                structured_requested_tiers,
                effective_tiers,
                conflicting_requested_tiers,
                filtered_out_candidates,
                filtered_out_tier_counts,
                top_provider_ids,
            } if pack_id == "spec-tool-search-conflicting-trust"
                && query == "trust:official search"
                && *returned == 0
                && *trust_filter_applied
                && query_requested_tiers == &vec!["official".to_owned()]
                && structured_requested_tiers == &vec!["verified-community".to_owned()]
                && effective_tiers.is_empty()
                && *conflicting_requested_tiers
                && *filtered_out_candidates == 2
                && filtered_out_tier_counts.get("official") == Some(&1)
                && filtered_out_tier_counts.get("verified-community") == Some(&1)
                && top_provider_ids.is_empty()
        )
    }));
}

#[tokio::test]
async fn execute_spec_tool_search_surfaces_slot_claim_activation_conflicts() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-tool-search-slot-conflict-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_a.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-a",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-a plugin");

    fs::write(
        plugin_root.join("search_b.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-b",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-tool-search-slot-conflict".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-tool-search-slot-conflict".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ToolSearch {
            query: "blocked_slot_claim_conflict".to_owned(),
            limit: 10,
            trust_tiers: Vec::new(),
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "tool_search");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 2);
    assert_eq!(report.outcome["returned"], 2);
    assert_eq!(
        report.outcome["results"][0]["activation_status"],
        "blocked_slot_claim_conflict"
    );
    assert!(
        report.outcome["results"][0]["activation_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("provider:web_search"))
    );
    assert!(
        report.outcome["results"][0]["diagnostic_findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| {
                finding["code"] == "slot_claim_conflict"
                    && finding["severity"] == "error"
                    && finding["phase"] == "activation"
                    && finding["blocking"] == true
            }))
    );
}

#[tokio::test]
async fn execute_spec_plugin_inventory_surfaces_activation_setup_and_ownership_truth() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-inventory-slot-conflict-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_a.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-a",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "setup": {
#     "mode": "metadata_only",
#     "surface": "web_search",
#     "required_env_vars": ["SEARCH_A_KEY"],
#     "required_config_keys": ["tools.web_search.default_provider"],
#     "default_env_var": "SEARCH_A_KEY",
#     "docs_urls": ["https://docs.example.com/search-a"],
#     "remediation": "configure search-a before enabling it"
#   },
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-a plugin");

    fs::write(
        plugin_root.join("search_b.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-b",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"tavily","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-slot-conflict".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-slot-conflict".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginInventory {
            query: "SEARCH_A_KEY".to_owned(),
            limit: 10,
            include_ready: false,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_inventory");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 2);
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(report.outcome["results"][0]["plugin_id"], "search-a");
    assert_eq!(
        report.outcome["results"][0]["activation_status"],
        "blocked_slot_claim_conflict"
    );
    assert_eq!(report.outcome["results"][0]["setup_surface"], "web_search");
    assert_eq!(
        report.outcome["results"][0]["setup_default_env_var"],
        "SEARCH_A_KEY"
    );
    assert_eq!(
        report.outcome["results"][0]["slot_claims"][0]["slot"],
        "provider:web_search"
    );
    assert!(
        report.outcome["results"][0]["bootstrap_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("register http connector adapter"))
    );
    assert!(
        report.outcome["results"][0]["diagnostic_findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| {
                finding["code"] == "slot_claim_conflict"
                    && finding["severity"] == "error"
                    && finding["phase"] == "activation"
                    && finding["blocking"] == true
            }))
    );
}

#[tokio::test]
async fn execute_spec_plugin_inventory_surfaces_host_compatibility_blockers() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-inventory-host-compat-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("incompatible_host.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "incompatible-host",
#   "provider_id": "incompatible-host",
#   "connector_name": "incompatible-host",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/incompatible-host",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","version":"1.0.0"},
#   "compatibility": {
#     "host_api": "loongclaw-plugin/v999",
#     "host_version_req": ">=0.1.0-alpha.1"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write incompatible-host plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-host-compat".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-host-compat".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginInventory {
            query: "blocked_incompatible_host".to_owned(),
            limit: 10,
            include_ready: false,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_inventory");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 1);
    assert!(
        report
            .plugin_absorb_reports
            .iter()
            .all(|absorb| absorb.absorbed_plugins == 0)
    );
    assert!(
        report
            .integration_catalog
            .provider("incompatible-host")
            .is_none()
    );
    assert_eq!(report.outcome["returned"], 1);
    assert_eq!(
        report.outcome["results"][0]["plugin_id"],
        "incompatible-host"
    );
    assert_eq!(
        report.outcome["results"][0]["activation_status"],
        "blocked_incompatible_host"
    );
    assert_eq!(
        report.outcome["results"][0]["compatibility"]["host_api"],
        "loongclaw-plugin/v999"
    );
    assert_eq!(
        report.outcome["results"][0]["compatibility"]["host_version_req"],
        ">=0.1.0-alpha.1"
    );
    assert!(
        report.outcome["results"][0]["activation_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("loongclaw-plugin/v1"))
    );
    assert!(
        report.outcome["results"][0]["diagnostic_findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| {
                finding["code"] == "incompatible_host"
                    && finding["severity"] == "error"
                    && finding["phase"] == "activation"
                    && finding["blocking"] == true
            }))
    );
}

#[tokio::test]
async fn execute_spec_plugin_inventory_requires_explicit_openclaw_shim_enablement() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-inventory-openclaw-shim-{unique}"));
    let package_root = plugin_root.join("weather-sdk");
    fs::create_dir_all(package_root.join("dist")).expect("create plugin root");

    fs::write(
        package_root.join("openclaw.plugin.json"),
        r#"
{
  "id": "weather-sdk",
  "name": "Weather SDK",
  "description": "OpenClaw weather integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["weather"],
  "channels": ["weather"],
  "skills": ["forecast"],
  "configSchema": {}
}
"#,
    )
    .expect("write openclaw manifest");
    fs::write(
        package_root.join("package.json"),
        r#"
{
  "name": "@acme/weather-sdk",
  "version": "1.2.3",
  "description": "Weather provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js",
    "channel": {
      "id": "weather",
      "label": "Weather",
      "aliases": ["forecast"]
    }
  }
}
"#,
    )
    .expect("write package.json");
    fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
    fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

    let bridge_support = BridgeSupportSpec {
        enabled: true,
        supported_bridges: vec![PluginBridgeKind::ProcessStdio],
        supported_adapter_families: Vec::new(),
        supported_compatibility_modes: vec![
            PluginCompatibilityMode::Native,
            PluginCompatibilityMode::OpenClawModern,
        ],
        supported_compatibility_shims: Vec::new(),
        supported_compatibility_shim_profiles: Vec::new(),
        enforce_supported: false,
        policy_version: None,
        expected_checksum: None,
        expected_sha256: None,
        execute_process_stdio: false,
        execute_http_json: false,
        allowed_process_commands: Vec::new(),
        enforce_execution_success: false,
        security_scan: None,
    };

    let blocked_spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-openclaw-shim-blocked".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-openclaw-shim-blocked".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(bridge_support.clone()),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginInventory {
            query: "weather-sdk".to_owned(),
            limit: 10,
            include_ready: true,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let blocked_report = execute_spec(&blocked_spec, true).await;
    assert_eq!(blocked_report.operation_kind, "plugin_inventory");
    assert!(blocked_report.blocked_reason.is_none());
    assert_eq!(blocked_report.outcome["returned"], 1);
    assert_eq!(
        blocked_report.outcome["results"][0]["activation_status"],
        "blocked_compatibility_mode"
    );
    assert_eq!(
        blocked_report.outcome["results"][0]["compatibility_mode"],
        "openclaw_modern"
    );
    assert_eq!(
        blocked_report.outcome["results"][0]["compatibility_shim"]["shim_id"],
        "openclaw-modern-compat"
    );
    assert!(
        blocked_report.outcome["results"][0]["activation_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("openclaw-modern-compat"))
    );

    let mut enabled_bridge_support = bridge_support.clone();
    enabled_bridge_support.supported_compatibility_shims =
        vec![loongclaw_daemon::kernel::PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        }];

    let enabled_spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-openclaw-shim-enabled".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-openclaw-shim-enabled".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(enabled_bridge_support),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: Some(PluginSetupReadinessSpec {
            inherit_process_env: false,
            verified_env_vars: Vec::new(),
            verified_config_keys: vec!["plugins.entries.weather-sdk".to_owned()],
        }),
        operation: OperationSpec::PluginInventory {
            query: "weather-sdk".to_owned(),
            limit: 10,
            include_ready: true,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let enabled_report = execute_spec(&enabled_spec, true).await;
    assert_eq!(enabled_report.operation_kind, "plugin_inventory");
    assert!(enabled_report.blocked_reason.is_none());
    assert_eq!(enabled_report.outcome["returned"], 1);
    assert_eq!(
        enabled_report.outcome["results"][0]["activation_status"],
        "ready"
    );
    assert_eq!(enabled_report.outcome["results"][0]["loaded"], json!(true));
    assert_eq!(
        enabled_report.outcome["results"][0]["compatibility_shim"]["shim_id"],
        "openclaw-modern-compat"
    );
    assert_eq!(
        enabled_report.outcome["results"][0]["activation_attestation"]["attested"],
        json!(true)
    );
    assert_eq!(
        enabled_report.outcome["results"][0]["activation_attestation"]["verified"],
        json!(true)
    );
    assert_eq!(
        enabled_report.outcome["results"][0]["activation_attestation"]["integrity"],
        json!("verified")
    );
    assert!(
        enabled_report.outcome["results"][0]["activation_attestation"]["checksum"]
            .as_str()
            .is_some_and(|checksum| !checksum.is_empty())
    );
}

#[tokio::test]
async fn execute_spec_plugin_inventory_blocks_openclaw_shim_profile_mismatch() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!(
        "loongclaw-plugin-inventory-openclaw-profile-{unique}"
    ));
    let package_root = plugin_root.join("weather-sdk");
    fs::create_dir_all(package_root.join("dist")).expect("create plugin root");

    fs::write(
        package_root.join("openclaw.plugin.json"),
        r#"
{
  "id": "weather-sdk",
  "name": "Weather SDK",
  "description": "OpenClaw weather integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["weather"],
  "channels": ["weather"],
  "skills": ["forecast"],
  "configSchema": {}
}
"#,
    )
    .expect("write openclaw manifest");
    fs::write(
        package_root.join("package.json"),
        r#"
{
  "name": "@acme/weather-sdk",
  "version": "1.2.3",
  "description": "Weather provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js",
    "channel": {
      "id": "weather",
      "label": "Weather",
      "aliases": ["forecast"]
    }
  }
}
"#,
    )
    .expect("write package.json");
    fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
    fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

    let bridge_support = BridgeSupportSpec {
        enabled: true,
        supported_bridges: vec![PluginBridgeKind::ProcessStdio],
        supported_adapter_families: Vec::new(),
        supported_compatibility_modes: vec![
            PluginCompatibilityMode::Native,
            PluginCompatibilityMode::OpenClawModern,
        ],
        supported_compatibility_shims: Vec::new(),
        supported_compatibility_shim_profiles: vec![PluginCompatibilityShimSupport {
            shim: PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            },
            version: Some("openclaw-modern@1".to_owned()),
            supported_dialects: BTreeSet::from([PluginContractDialect::OpenClawModernManifest]),
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_source_languages: BTreeSet::from(["python".to_owned()]),
        }],
        enforce_supported: false,
        policy_version: None,
        expected_checksum: None,
        expected_sha256: None,
        execute_process_stdio: false,
        execute_http_json: false,
        allowed_process_commands: Vec::new(),
        enforce_execution_success: false,
        security_scan: None,
    };

    let blocked_spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-openclaw-profile-blocked".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-openclaw-profile-blocked".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(bridge_support.clone()),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginInventory {
            query: "weather-sdk".to_owned(),
            limit: 10,
            include_ready: true,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let blocked_report = execute_spec(&blocked_spec, true).await;
    assert_eq!(blocked_report.operation_kind, "plugin_inventory");
    assert!(blocked_report.blocked_reason.is_none());
    assert_eq!(blocked_report.outcome["returned"], 1);
    assert_eq!(
        blocked_report.outcome["results"][0]["activation_status"],
        "blocked_compatibility_mode"
    );
    assert!(
        blocked_report.outcome["results"][0]["activation_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("source language `javascript`"))
    );
    assert!(
        blocked_report.outcome["results"][0]["activation_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("openclaw-modern@1"))
    );

    let mut enabled_bridge_support = bridge_support.clone();
    enabled_bridge_support.supported_compatibility_shim_profiles[0].supported_source_languages =
        BTreeSet::from(["javascript".to_owned()]);

    let enabled_spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-inventory-openclaw-profile-enabled".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-inventory-openclaw-profile-enabled".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(enabled_bridge_support),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: Some(PluginSetupReadinessSpec {
            inherit_process_env: false,
            verified_env_vars: Vec::new(),
            verified_config_keys: vec!["plugins.entries.weather-sdk".to_owned()],
        }),
        operation: OperationSpec::PluginInventory {
            query: "weather-sdk".to_owned(),
            limit: 10,
            include_ready: true,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let enabled_report = execute_spec(&enabled_spec, true).await;
    assert_eq!(enabled_report.operation_kind, "plugin_inventory");
    assert!(enabled_report.blocked_reason.is_none());
    assert_eq!(enabled_report.outcome["returned"], 1);
    assert_eq!(
        enabled_report.outcome["results"][0]["activation_status"],
        "ready"
    );
    assert_eq!(
        enabled_report.outcome["results"][0]["compatibility_shim"]["shim_id"],
        "openclaw-modern-compat"
    );
}

#[tokio::test]
async fn execute_spec_openclaw_connector_runtime_surfaces_attested_activation_contract() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-openclaw-runtime-attested-{unique}"));
    let package_root = plugin_root.join("weather-sdk");
    fs::create_dir_all(package_root.join("dist")).expect("create plugin root");

    fs::write(
        package_root.join("openclaw.plugin.json"),
        r#"
{
  "id": "weather-sdk",
  "name": "Weather SDK",
  "description": "OpenClaw weather integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["weather"],
  "channels": ["weather"],
  "skills": ["forecast"],
  "configSchema": {}
}
"#,
    )
    .expect("write openclaw manifest");
    fs::write(
        package_root.join("package.json"),
        r#"
{
  "name": "@acme/weather-sdk",
  "version": "1.2.3",
  "description": "Weather provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "channel": {
      "id": "weather",
      "label": "Weather",
      "aliases": ["forecast"]
    }
  }
}
"#,
    )
    .expect("write package.json");
    fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-openclaw-runtime-attested".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-openclaw-runtime-attested".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: Some(BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::ProcessStdio],
            supported_adapter_families: Vec::new(),
            supported_compatibility_modes: vec![
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ],
            supported_compatibility_shims: Vec::new(),
            supported_compatibility_shim_profiles: vec![PluginCompatibilityShimSupport {
                shim: PluginCompatibilityShim {
                    shim_id: "openclaw-modern-compat".to_owned(),
                    family: "openclaw-modern-compat".to_owned(),
                },
                version: Some("openclaw-modern@1".to_owned()),
                supported_dialects: BTreeSet::from([PluginContractDialect::OpenClawModernManifest]),
                supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                supported_adapter_families: BTreeSet::new(),
                supported_source_languages: BTreeSet::from(["javascript".to_owned()]),
            }],
            enforce_supported: true,
            policy_version: None,
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: Some(BootstrapSpec {
            enabled: true,
            allow_http_json_auto_apply: Some(false),
            allow_process_stdio_auto_apply: Some(true),
            allow_native_ffi_auto_apply: Some(false),
            allow_wasm_component_auto_apply: Some(false),
            allow_mcp_server_auto_apply: Some(false),
            allow_acp_bridge_auto_apply: Some(false),
            allow_acp_runtime_auto_apply: Some(false),
            block_unverified_high_risk_auto_apply: None,
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: Some(AutoProvisionSpec {
            enabled: true,
            provider_id: "weather-sdk".to_owned(),
            channel_id: "primary".to_owned(),
            connector_name: Some("weather-sdk".to_owned()),
            endpoint: Some("stdio://weather-sdk".to_owned()),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        }),
        hotfixes: Vec::new(),
        plugin_setup_readiness: Some(PluginSetupReadinessSpec {
            inherit_process_env: false,
            verified_env_vars: Vec::new(),
            verified_config_keys: vec!["plugins.entries.weather-sdk".to_owned()],
        }),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "weather-sdk".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"city":"shanghai"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "planned"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["runtime_guard"]
            ["activation_contract_attested"],
        json!(true)
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["runtime_guard"]
            ["activation_contract_verified"],
        json!(true)
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["runtime_guard"]
            ["activation_contract_integrity"],
        json!("verified")
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["shim_support"]
            ["version"],
        json!("openclaw-modern@1")
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["activation_contract"]
            ["plugin_id"],
        json!("weather-sdk")
    );

    let provider = report
        .integration_catalog
        .provider("weather-sdk")
        .expect("provider should be absorbed");
    let raw_contract = provider
        .metadata
        .get("plugin_activation_contract_json")
        .expect("provider metadata should carry activation contract");
    let metadata_checksum = provider
        .metadata
        .get("plugin_activation_contract_checksum")
        .cloned()
        .expect("provider metadata should carry activation contract checksum");
    let contract_value: serde_json::Value =
        serde_json::from_str(raw_contract).expect("activation contract should decode");
    assert_eq!(
        contract_value["compatibility_mode"],
        json!("openclaw_modern")
    );
    assert_eq!(
        contract_value["compatibility_shim"]["shim_id"],
        json!("openclaw-modern-compat")
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["plugin_compatibility"]["activation_contract_checksum"],
        json!(metadata_checksum)
    );
}

#[tokio::test]
async fn execute_spec_plugin_preflight_summarizes_runtime_activation_blockers() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-preflight-runtime-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_a.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-a",
#   "provider_id": "search-a",
#   "connector_name": "search-a",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-a",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"default","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-a plugin");

    fs::write(
        plugin_root.join("search_b.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-b",
#   "provider_id": "search-b",
#   "connector_name": "search-b",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-b",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"},
#   "slot_claims": [
#     {"slot":"provider:web_search","key":"default","mode":"exclusive"}
#   ]
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-b plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-preflight-runtime".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-preflight-runtime".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginPreflight {
            query: String::new(),
            limit: 10,
            profile: PluginPreflightProfile::RuntimeActivation,
            policy_path: None,
            policy_sha256: None,
            policy_signature: None,
            include_passed: false,
            include_warned: false,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_preflight");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.outcome["summary"]["profile"], "runtime_activation");
    assert_eq!(
        report.outcome["summary"]["policy_source"],
        "bundled:plugin-preflight-medium-balanced.json"
    );
    assert_eq!(report.outcome["summary"]["matched_plugins"], 2);
    assert_eq!(report.outcome["summary"]["baseline_blocked_plugins"], 2);
    assert_eq!(report.outcome["summary"]["blocked_plugins"], 2);
    assert_eq!(
        report.outcome["summary"]["findings_by_code"]["slot_claim_conflict"],
        2
    );
    assert_eq!(
        report.outcome["summary"]["remediation_counts"]["resolve_slot_ownership_conflict"],
        2
    );
    assert_eq!(
        report.outcome["summary"]["operator_action_counts_by_surface"]["plugin_package"],
        4
    );
    assert_eq!(
        report.outcome["summary"]["operator_action_counts_by_kind"]["resolve_slot_ownership"],
        2
    );
    assert_eq!(
        report.outcome["summary"]["operator_action_counts_by_kind"]["update_plugin_package"],
        2
    );
    assert_eq!(
        report.outcome["summary"]["operator_actions_requiring_reload"],
        4
    );
    assert_eq!(
        report.outcome["summary"]["operator_actions_without_reload"],
        0
    );
    assert!(
        report.outcome["summary"]["operator_action_plan"]
            .as_array()
            .is_some_and(|plan| {
                plan.len() == 4
                    && plan.iter().all(|item| {
                        item["action"]["action_id"]
                            .as_str()
                            .is_some_and(|action_id| action_id.len() == 64)
                            && item["supporting_results"] == 1
                            && item["blocked_results"] == 1
                            && item["warned_results"] == 0
                            && item["passed_results"] == 0
                            && item["supporting_remediations"]
                                .as_array()
                                .is_some_and(|supports| !supports.is_empty())
                    })
                    && plan
                        .iter()
                        .filter(|item| {
                            item["action"]["kind"] == "resolve_slot_ownership"
                                && item["supporting_remediations"]
                                    .as_array()
                                    .is_some_and(|supports| supports.len() == 1)
                        })
                        .count()
                        == 2
                    && plan
                        .iter()
                        .filter(|item| {
                            item["action"]["kind"] == "update_plugin_package"
                                && item["supporting_remediations"]
                                    .as_array()
                                    .is_some_and(|supports| supports.len() == 2)
                        })
                        .count()
                        == 2
            })
    );
    assert_eq!(report.outcome["returned"], 2);
    assert_eq!(report.outcome["results"][0]["baseline_verdict"], "block");
    assert_eq!(report.outcome["results"][0]["verdict"], "block");
    assert!(
        report.outcome["results"][0]["policy_flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|flag| flag == "activation_blocked"))
    );
    assert!(
        report.outcome["results"][0]["blocking_diagnostic_codes"]
            .as_array()
            .is_some_and(|codes| codes.iter().any(|code| code == "slot_claim_conflict"))
    );
    assert!(
        report.outcome["results"][0]["recommended_actions"]
            .as_array()
            .is_some_and(|actions| actions.iter().any(|action| {
                action["remediation_class"] == "resolve_slot_ownership_conflict"
                    && action["operator_action"]["surface"] == "plugin_package"
                    && action["operator_action"]["kind"] == "resolve_slot_ownership"
                    && action["operator_action"]["action_id"]
                        .as_str()
                        .is_some_and(|action_id| action_id.len() == 64)
                    && action["operator_action"]["follow_up_profile"] == "runtime_activation"
                    && action["operator_action"]["requires_reload"] == true
            }))
    );
}

#[tokio::test]
async fn execute_spec_plugin_preflight_blocks_embedded_source_sdk_release_contracts() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-preflight-sdk-release-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_sdk.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-sdk",
#   "version": "0.1.0",
#   "provider_id": "search-sdk",
#   "connector_name": "search-sdk",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-sdk",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-sdk plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-preflight-sdk-release".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-preflight-sdk-release".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginPreflight {
            query: String::new(),
            limit: 10,
            profile: PluginPreflightProfile::SdkRelease,
            policy_path: None,
            policy_sha256: None,
            policy_signature: None,
            include_passed: false,
            include_warned: false,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_preflight");
    assert!(report.blocked_reason.is_none());
    assert_eq!(report.outcome["summary"]["profile"], "sdk_release");
    assert_eq!(
        report.outcome["summary"]["policy_source"],
        "bundled:plugin-preflight-medium-balanced.json"
    );
    assert_eq!(report.outcome["summary"]["matched_plugins"], 1);
    assert_eq!(report.outcome["summary"]["baseline_blocked_plugins"], 1);
    assert_eq!(report.outcome["summary"]["blocked_plugins"], 1);
    assert_eq!(report.outcome["summary"]["blocking_diagnostics"], 0);
    assert_eq!(
        report.outcome["summary"]["findings_by_code"]["embedded_source_legacy_contract"],
        1
    );
    assert_eq!(
        report.outcome["summary"]["remediation_counts"]["migrate_to_package_manifest"],
        1
    );
    assert_eq!(report.outcome["results"][0]["baseline_verdict"], "block");
    assert_eq!(report.outcome["results"][0]["verdict"], "block");
    assert_eq!(report.outcome["results"][0]["activation_ready"], true);
    assert_eq!(
        report.outcome["results"][0]["plugin"]["source_kind"],
        "embedded_source"
    );
    assert!(
        report.outcome["results"][0]["policy_flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|flag| flag == "embedded_source_contract"))
    );
}

#[tokio::test]
async fn execute_spec_plugin_preflight_honors_custom_policy_path_and_sha() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-preflight-custom-policy-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_sdk.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-sdk",
#   "provider_id": "search-sdk",
#   "connector_name": "search-sdk",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-sdk",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-sdk plugin");

    let policy = PluginPreflightPolicyProfile {
        policy_version: Some("custom-runtime-gate".to_owned()),
        runtime_activation: PluginPreflightRuleProfile {
            block_on_embedded_source_contract: true,
            ..PluginPreflightRuleProfile::default()
        },
        ..PluginPreflightPolicyProfile::default()
    };
    let policy_path =
        std::env::temp_dir().join(format!("loongclaw-plugin-preflight-policy-{unique}.json"));
    fs::write(
        &policy_path,
        serde_json::to_string_pretty(&policy).expect("encode policy"),
    )
    .expect("write custom policy");
    let policy_sha256 = plugin_preflight_policy_sha256(&policy);

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-preflight-custom-policy".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-preflight-custom-policy".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginPreflight {
            query: String::new(),
            limit: 10,
            profile: PluginPreflightProfile::RuntimeActivation,
            policy_path: Some(policy_path.display().to_string()),
            policy_sha256: Some(policy_sha256.clone()),
            policy_signature: None,
            include_passed: false,
            include_warned: false,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_preflight");
    assert!(report.blocked_reason.is_none());
    assert_eq!(
        report.outcome["summary"]["policy_source"],
        policy_path.display().to_string()
    );
    assert_eq!(
        report.outcome["summary"]["policy_version"],
        "custom-runtime-gate"
    );
    assert_eq!(report.outcome["summary"]["policy_sha256"], policy_sha256);
    assert_eq!(report.outcome["summary"]["blocked_plugins"], 1);
    assert_eq!(report.outcome["results"][0]["baseline_verdict"], "block");
    assert_eq!(report.outcome["results"][0]["verdict"], "block");
    assert_eq!(
        report.outcome["results"][0]["plugin"]["source_kind"],
        "embedded_source"
    );
}

#[tokio::test]
async fn execute_spec_plugin_preflight_applies_contract_drift_exception_lane() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-plugin-preflight-waiver-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("search_sdk.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "search-sdk",
#   "provider_id": "search-sdk",
#   "connector_name": "search-sdk",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/search-sdk",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write search-sdk plugin");

    let policy = PluginPreflightPolicyProfile {
        policy_version: Some("private-sdk-exception-lane".to_owned()),
        exceptions: vec![PluginPreflightPolicyException {
            exception_id: "grandfather-search-sdk".to_owned(),
            plugin_id: "search-sdk".to_owned(),
            plugin_version_req: None,
            profiles: vec![PluginPreflightProfile::SdkRelease],
            waive_policy_flags: vec!["embedded_source_contract".to_owned()],
            waive_diagnostic_codes: vec!["embedded_source_legacy_contract".to_owned()],
            reason: "private registry migration window".to_owned(),
            ticket_ref: "SEC-1001".to_owned(),
            approved_by: "platform-security".to_owned(),
            expires_at: Some("2026-06-30".to_owned()),
        }],
        ..PluginPreflightPolicyProfile::default()
    };
    let policy_path = std::env::temp_dir().join(format!(
        "loongclaw-plugin-preflight-waiver-policy-{unique}.json"
    ));
    fs::write(
        &policy_path,
        serde_json::to_string_pretty(&policy).expect("encode policy"),
    )
    .expect("write custom policy");
    let policy_sha256 = plugin_preflight_policy_sha256(&policy);

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-plugin-preflight-waiver".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-plugin-preflight-waiver".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
            ..HumanApprovalSpec::default()
        }),
        defaults: None,
        self_awareness: None,
        plugin_scan: Some(PluginScanSpec {
            enabled: true,
            roots: vec![plugin_root.display().to_string()],
        }),
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::PluginPreflight {
            query: String::new(),
            limit: 10,
            profile: PluginPreflightProfile::SdkRelease,
            policy_path: Some(policy_path.display().to_string()),
            policy_sha256: Some(policy_sha256.clone()),
            policy_signature: None,
            include_passed: true,
            include_warned: true,
            include_blocked: true,
            include_deferred: true,
            include_examples: false,
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "plugin_preflight");
    assert!(report.blocked_reason.is_none());
    assert_eq!(
        report.outcome["summary"]["policy_source"],
        policy_path.display().to_string()
    );
    assert_eq!(
        report.outcome["summary"]["policy_version"],
        "private-sdk-exception-lane"
    );
    assert_eq!(report.outcome["summary"]["policy_sha256"], policy_sha256);
    assert_eq!(report.outcome["summary"]["baseline_blocked_plugins"], 1);
    assert_eq!(report.outcome["summary"]["clean_passed_plugins"], 0);
    assert_eq!(report.outcome["summary"]["waived_passed_plugins"], 1);
    assert_eq!(report.outcome["summary"]["passed_plugins"], 1);
    assert_eq!(report.outcome["summary"]["waived_plugins"], 1);
    assert_eq!(report.outcome["summary"]["applied_exception_count"], 1);
    assert_eq!(
        report.outcome["summary"]["exception_counts_by_ticket"]["SEC-1001"],
        1
    );
    assert_eq!(
        report.outcome["summary"]["exception_counts_by_approver"]["platform-security"],
        1
    );
    assert_eq!(
        report.outcome["summary"]["waived_policy_flags"]["embedded_source_contract"],
        1
    );
    assert_eq!(
        report.outcome["summary"]["waived_diagnostic_codes"]["embedded_source_legacy_contract"],
        1
    );
    assert_eq!(
        report.outcome["summary"]["remediation_counts"]["migrate_to_package_manifest"],
        1
    );
    assert_eq!(report.outcome["results"][0]["baseline_verdict"], "block");
    assert_eq!(report.outcome["results"][0]["verdict"], "pass");
    assert_eq!(report.outcome["results"][0]["exception_applied"], true);
    assert!(
        report.outcome["results"][0]["policy_flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().any(|flag| flag == "embedded_source_contract"))
    );
    assert!(
        report.outcome["results"][0]["effective_policy_flags"]
            .as_array()
            .is_some_and(|flags| flags.iter().all(|flag| flag != "embedded_source_contract"))
    );
    assert_eq!(
        report.outcome["results"][0]["applied_exceptions"][0]["exception_id"],
        "grandfather-search-sdk"
    );
    assert_eq!(
        report.outcome["results"][0]["applied_exceptions"][0]["ticket_ref"],
        "SEC-1001"
    );
}
