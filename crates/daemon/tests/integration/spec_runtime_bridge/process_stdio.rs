use super::*;
use loong_daemon::kernel::PluginCompatibilityMode;
use std::sync::MutexGuard;

struct ProcessStdioEnvironmentGuard {
    _lock: MutexGuard<'static, ()>,
}

impl ProcessStdioEnvironmentGuard {
    fn new() -> Self {
        Self {
            _lock: lock_daemon_test_environment(),
        }
    }
}

#[tokio::test]
async fn execute_spec_process_stdio_bridge_executes_when_enabled_and_allowed() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loong-plugin-process-stdio-run-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-plugin",
#   "provider_id": "stdio-provider",
#   "connector_name": "stdio-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "process_timeout_ms":"999999999",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-run".to_owned(),
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
        agent_id: "agent-process-stdio-run".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
            enforce_execution_success: true,
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "executed"
    );
    assert_process_stdio_runtime_shape(runtime);
    assert_process_stdio_runtime_keys(
        runtime,
        &[
            "exit_code",
            "response_id",
            "response_method",
            "stderr",
            "stdout",
            "stdout_json",
        ],
    );
    assert_eq!(runtime["stdout_json"]["operation"], "invoke");
    assert!(runtime.get("exit_code").is_some());
    assert!(runtime.get("response_method").is_some());
    assert!(runtime.get("response_id").is_some());
    assert_bridge_runtime_protocol_context(
        runtime,
        "stdio-provider:primary:invoke",
        "invoke",
        "invoke",
    );
}

#[tokio::test]
async fn execute_spec_process_stdio_bridge_blocks_when_command_not_allowlisted() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-block-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-plugin",
#   "provider_id": "stdio-provider",
#   "connector_name": "stdio-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "process_timeout_ms":"999999999",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-block".to_owned(),
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
        agent_id: "agent-process-stdio-block".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["python3".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
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
            .contains("not allowed")
    );
}

#[tokio::test]
async fn execute_spec_process_stdio_bridge_fails_on_invalid_json_line_response() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-invalid-frame-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-invalid-frame-plugin",
#   "provider_id": "stdio-invalid-frame-provider",
#   "connector_name": "stdio-invalid-frame-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-invalid-frame-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"printf",
#     "args_json":"[\"not-json\\n\"]",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-invalid-frame".to_owned(),
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
        agent_id: "agent-process-stdio-invalid-frame".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["printf".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-invalid-frame-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    let reason = report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
        .as_str()
        .expect("failed reason should be string");
    assert!(
        reason.contains("failed to decode inbound frame")
            || reason.contains("failed to read frame")
            || reason.contains("failed to write frame"),
        "unexpected failure reason: {reason}"
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["transport_kind"],
        "json_line"
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn execute_spec_process_stdio_bridge_strips_high_risk_env_vars_from_child_process() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut env = loong_daemon::test_support::ScopedEnv::new();
    env.set("CC", "malicious-cc");
    env.set("PYTHONPATH", "/tmp/malicious-pythonpath");
    env.set("LC_CHILD_TEST", "keep-me");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-env-sanitized-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-env-sanitized-plugin",
#   "provider_id": "stdio-env-sanitized-provider",
#   "connector_name": "stdio-env-sanitized-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-env-sanitized-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "process_timeout_ms":"15000",
#     "args_json":"[\"-c\",\"import json, os, sys; request=json.loads(sys.stdin.readline()); payload={'cc': os.environ.get('CC', 'unset'), 'pythonpath': os.environ.get('PYTHONPATH', 'unset'), 'lc_child_test': os.environ.get('LC_CHILD_TEST', 'unset'), 'path': 'set' if os.environ.get('PATH') else 'unset'}; response={'method': request['method'], 'id': request['id'], 'payload': payload}; sys.stdout.write(json.dumps(response)+'\\\\n'); sys.stdout.flush()\"]",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-env-sanitized".to_owned(),
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
        agent_id: "agent-process-stdio-env-sanitized".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["python3".to_owned()],
            enforce_execution_success: true,
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-env-sanitized-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    let bridge_execution = &report.outcome["outcome"]["payload"]["bridge_execution"];
    let response_payload = &bridge_execution["runtime"]["stdout_json"];

    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(bridge_execution["status"], "executed");
    assert_eq!(response_payload["cc"], "unset");
    assert_eq!(response_payload["pythonpath"], "unset");
    assert_eq!(response_payload["lc_child_test"], "keep-me");
    assert_eq!(response_payload["path"], "set");
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn execute_spec_process_stdio_bridge_fails_on_response_id_mismatch() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-mismatch-id-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-mismatch-id-plugin",
#   "provider_id": "stdio-mismatch-id-provider",
#   "connector_name": "stdio-mismatch-id-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-mismatch-id-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "process_timeout_ms":"15000",
#     "args_json":"[\"-c\",\"import json,sys; request=json.loads(sys.stdin.readline()); response={'method':request['method'],'id':'wrong-id','payload':{'ok':True}}; sys.stdout.write(json.dumps(response)+'\\\\n'); sys.stdout.flush()\"]",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-mismatch-id".to_owned(),
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
        agent_id: "agent-process-stdio-mismatch-id".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["python3".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-mismatch-id-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    let reason = report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
        .as_str()
        .expect("failed reason should be string");
    assert!(
        reason.contains("response id mismatch"),
        "unexpected bridge failure reason: {reason}"
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn execute_spec_process_stdio_bridge_fails_on_response_method_mismatch() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!(
        "loong-plugin-process-stdio-mismatch-method-{unique}"
    ));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-mismatch-method-plugin",
#   "provider_id": "stdio-mismatch-method-provider",
#   "connector_name": "stdio-mismatch-method-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-mismatch-method-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "process_timeout_ms":"15000",
#     "args_json":"[\"-c\",\"import json,sys; request=json.loads(sys.stdin.readline()); response={'method':'tools/list','id':request['id'],'payload':{'ok':True}}; sys.stdout.write(json.dumps(response)+'\\\\n'); sys.stdout.flush()\"]",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-mismatch-method".to_owned(),
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
        agent_id: "agent-process-stdio-mismatch-method".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["python3".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-mismatch-method-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    let reason = report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
        .as_str()
        .expect("failed reason should be string");
    assert!(
        reason.contains("response method mismatch"),
        "unexpected bridge failure reason: {reason}"
    );
}

#[tokio::test]
async fn execute_spec_process_stdio_bridge_blocks_when_protocol_authorization_fails() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-authz-block-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-authz-block-plugin",
#   "provider_id": "stdio-authz-block-provider",
#   "connector_name": "stdio-authz-block-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-authz-block-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "process_timeout_ms":"999999999",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-authz-block".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::MemoryRead]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-process-stdio-authz-block".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["cat".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-authz-block-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::MemoryRead]),
            payload: json!({"question":"ping"}),
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
            .contains("protocol route authorization failed")
    );
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];
    assert_process_stdio_runtime_shape(runtime);
    assert_process_stdio_runtime_keys(runtime, &[]);
    assert!(runtime.get("exit_code").is_none());
    assert!(runtime.get("response_method").is_none());
    assert!(runtime.get("response_id").is_none());
    assert_bridge_runtime_protocol_context(
        runtime,
        "stdio-authz-block-provider:primary:invoke",
        "invoke",
        "discover",
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["timeout_ms"],
        300000
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn execute_spec_process_stdio_bridge_fails_on_recv_timeout() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let _env_lock = ProcessStdioEnvironmentGuard::new();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loong-plugin-process-stdio-timeout-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("stdio_plugin.py"),
        r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "stdio-timeout-plugin",
#   "provider_id": "stdio-timeout-provider",
#   "connector_name": "stdio-timeout-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-timeout-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"sleep",
#     "args_json":"[\"0.2\"]",
#     "process_timeout_ms":"50",
#     "version":"1.0.0"
#   }
# }
# LOONG_PLUGIN_END
"#,
    )
    .expect("write stdio plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-process-stdio-timeout".to_owned(),
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
        agent_id: "agent-process-stdio-timeout".to_owned(),
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
            execute_process_stdio: true,
            execute_http_json: false,
            allowed_process_commands: vec!["sleep".to_owned()],
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
            max_tasks: Some(10),
        }),
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "stdio-timeout-provider".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"question":"ping"}),
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(report.outcome["outcome"]["status"], "ok");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("failed reason should be string")
            .contains("timed out")
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["timeout_ms"],
        50
    );
}
