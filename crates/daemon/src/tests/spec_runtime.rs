use super::*;

#[test]
fn template_spec_is_json_roundtrip_stable() {
    let spec = RunnerSpec::template();
    let encoded = serde_json::to_string_pretty(&spec).expect("encode spec");
    let decoded: RunnerSpec = serde_json::from_str(&encoded).expect("decode spec");
    assert_eq!(decoded.pack.pack_id, "sales-intel-local");
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
        report
            .integration_catalog
            .provider("ffi-provider")
            .expect("provider should exist")
            .metadata
            .get("bridge_kind")
            .cloned(),
        Some("native_ffi".to_owned())
    );
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
    assert!(provider.metadata.contains_key("plugin_source_path"));
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(5),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
async fn execute_spec_tool_core_can_run_claw_import_plan_via_native_tool_runtime() {
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

    let root = unique_temp_dir("loongclaw-spec-tool-core-import");
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
            pack_id: "spec-tool-core-claw-import".to_owned(),
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
        agent_id: "agent-tool-core-claw-import".to_owned(),
        ttl_s: 120,
        approval: None,
        defaults: None,
        self_awareness: None,
        plugin_scan: None,
        bridge_support: None,
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ToolCore {
            tool_name: "claw.import".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({
                "mode": "plan",
                "source": "nanobot",
                "input_path": root.display().to_string()
            }),
            core: None,
        },
    };

    let report = execute_spec(&spec, true).await;
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
async fn execute_spec_tool_extension_can_hot_handle_claw_import_via_core_wrapper() {
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

    let report = execute_spec(&spec, true).await;
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

    let report = execute_spec(&spec, true).await;
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

    let report = execute_spec(&spec, true).await;
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

    let report = execute_spec(&spec, true).await;
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(true),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
        operation: OperationSpec::Task {
            task_id: "t-plugin-rollback".to_owned(),
            objective: "must block and rollback staged plugin absorb".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "blocked");
    assert!(
        report
            .blocked_reason
            .expect("blocked reason")
            .contains("bridge support enforcement blocked")
    );
    assert_eq!(report.plugin_scan_reports.len(), 2);
    assert!(report.plugin_absorb_reports.is_empty());
    assert!(report.integration_catalog.provider("rollback-a").is_none());
    assert!(report.integration_catalog.provider("rollback-b").is_none());
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(1),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ToolSearch {
            query: "web search".to_owned(),
            limit: 5,
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
            enforce_ready_execution: Some(false),
            max_tasks: Some(8),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ToolSearch {
            query: "rusty".to_owned(),
            limit: 5,
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
