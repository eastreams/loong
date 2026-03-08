use super::*;

#[tokio::test]
async fn execute_spec_http_json_bridge_executes_against_local_server() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 4096];
            let _ = stream.read(&mut request_buf);
            let body = r#"{"status":"ok","reply":"pong"}"#;
            let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-http-runtime-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_plugin.py"),
        format!(
            r#"
# LOONGCLAW_PLUGIN_START
# {{
#   "plugin_id": "http-runtime",
#   "provider_id": "http-runtime",
#   "connector_name": "http-runtime",
#   "channel_id": "primary",
#   "endpoint": "http://{addr}/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {{
#     "bridge_kind":"http_json",
#     "http_method":"POST",
#     "http_timeout_ms":"3000",
#     "version":"1.0.0"
#   }}
# }}
# LOONGCLAW_PLUGIN_END
"#
        ),
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-http-runtime".to_owned(),
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
        agent_id: "agent-http-runtime".to_owned(),
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
            execute_http_json: true,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: true,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-runtime".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"query":"ping"}),
        },
    };

    let report = execute_spec(spec, true).await;
    server.join().expect("join local http server");
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];

    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "executed"
    );
    assert_http_json_runtime_shape(runtime);
    assert_http_json_runtime_keys(
        runtime,
        &["request", "response_json", "response_text", "status_code"],
    );
    assert_eq!(runtime["response_json"]["reply"], "pong");
    assert!(runtime.get("status_code").is_some());
    assert!(runtime.get("response_text").is_some());
    assert!(runtime.get("response_method").is_none());
    assert!(runtime.get("response_id").is_none());
    assert_bridge_runtime_protocol_context(
        runtime,
        "http-runtime:primary:invoke",
        "invoke",
        "invoke",
    );
}

#[tokio::test]
async fn execute_spec_http_json_bridge_blocks_when_protocol_authorization_fails() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-http-authz-block-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "http-authz-block",
#   "provider_id": "http-authz-block",
#   "connector_name": "http-authz-block",
#   "channel_id": "primary",
#   "endpoint": "http://127.0.0.1:9/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"http_json",
#     "http_method":"POST",
#     "http_timeout_ms":"999999999",
#     "version":"1.0.0"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-http-authz-block".to_owned(),
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
        agent_id: "agent-http-authz-block".to_owned(),
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
            execute_http_json: true,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-authz-block".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::MemoryRead]),
            payload: json!({"query":"ping"}),
        },
    };

    let report = execute_spec(spec, true).await;
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];
    assert_eq!(report.operation_kind, "connector_legacy");
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
    assert_http_json_runtime_shape(runtime);
    assert_http_json_runtime_keys(runtime, &[]);
    assert!(runtime.get("status_code").is_none());
    assert!(runtime.get("response_text").is_none());
    assert!(runtime.get("response_json").is_none());
    assert!(runtime.get("response_method").is_none());
    assert!(runtime.get("response_id").is_none());
    assert_bridge_runtime_protocol_context(
        runtime,
        "http-authz-block:primary:invoke",
        "invoke",
        "discover",
    );
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["timeout_ms"],
        300000
    );
}

#[tokio::test]
async fn execute_spec_http_json_bridge_strict_contract_fails_on_method_mismatch() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 4096];
            let _ = stream.read(&mut request_buf);
            let body = r#"{"method":"tools/list","id":"http-strict-method:primary:invoke","payload":{"reply":"pong"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-http-strict-method-mismatch-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_plugin.py"),
        format!(
            r#"
# LOONGCLAW_PLUGIN_START
# {{
#   "plugin_id": "http-strict-method",
#   "provider_id": "http-strict-method",
#   "connector_name": "http-strict-method",
#   "channel_id": "primary",
#   "endpoint": "http://{addr}/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {{
#     "bridge_kind":"http_json",
#     "http_method":"POST",
#     "http_enforce_protocol_contract":"true",
#     "http_timeout_ms":"3000",
#     "version":"1.0.0"
#   }}
# }}
# LOONGCLAW_PLUGIN_END
"#
        ),
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-http-strict-method".to_owned(),
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
        agent_id: "agent-http-strict-method".to_owned(),
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
            execute_http_json: true,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-strict-method".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"query":"ping"}),
        },
    };

    let report = execute_spec(spec, true).await;
    server.join().expect("join local http server");
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];

    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("failed reason should be string")
            .contains("response method mismatch")
    );
    assert_http_json_runtime_keys(runtime, &["request"]);
}

#[tokio::test]
async fn execute_spec_http_json_bridge_strict_contract_fails_on_id_mismatch() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 4096];
            let _ = stream.read(&mut request_buf);
            let body = r#"{"method":"tools/call","id":"http-strict-id:primary:other","payload":{"reply":"pong"}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root =
        std::env::temp_dir().join(format!("loongclaw-http-strict-id-mismatch-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_plugin.py"),
        format!(
            r#"
# LOONGCLAW_PLUGIN_START
# {{
#   "plugin_id": "http-strict-id",
#   "provider_id": "http-strict-id",
#   "connector_name": "http-strict-id",
#   "channel_id": "primary",
#   "endpoint": "http://{addr}/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {{
#     "bridge_kind":"http_json",
#     "http_method":"POST",
#     "http_enforce_protocol_contract":"true",
#     "http_timeout_ms":"3000",
#     "version":"1.0.0"
#   }}
# }}
# LOONGCLAW_PLUGIN_END
"#
        ),
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-http-strict-id".to_owned(),
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
        agent_id: "agent-http-strict-id".to_owned(),
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
            execute_http_json: true,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-strict-id".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"query":"ping"}),
        },
    };

    let report = execute_spec(spec, true).await;
    server.join().expect("join local http server");
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];

    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "failed"
    );
    assert!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
            .as_str()
            .expect("failed reason should be string")
            .contains("response id mismatch")
    );
    assert_http_json_runtime_keys(runtime, &["request"]);
}

#[tokio::test]
async fn execute_spec_http_json_bridge_strict_contract_executes_on_matching_frame() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
    let addr = listener.local_addr().expect("local addr");
    let expected_id = "http-strict-pass:primary:invoke".to_owned();
    let expected_id_for_server = expected_id.clone();
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 4096];
            let _ = stream.read(&mut request_buf);
            let body = format!(
                "{{\"method\":\"tools/call\",\"id\":\"{}\",\"payload\":{{\"reply\":\"pong\"}}}}",
                expected_id_for_server
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-http-strict-pass-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("http_plugin.py"),
        format!(
            r#"
# LOONGCLAW_PLUGIN_START
# {{
#   "plugin_id": "http-strict-pass",
#   "provider_id": "http-strict-pass",
#   "connector_name": "http-strict-pass",
#   "channel_id": "primary",
#   "endpoint": "http://{addr}/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {{
#     "bridge_kind":"http_json",
#     "http_method":"POST",
#     "http_enforce_protocol_contract":"true",
#     "http_timeout_ms":"3000",
#     "version":"1.0.0"
#   }}
# }}
# LOONGCLAW_PLUGIN_END
"#
        ),
    )
    .expect("write http plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-http-strict-pass".to_owned(),
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
        agent_id: "agent-http-strict-pass".to_owned(),
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
            execute_http_json: true,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: true,
            security_scan: None,
        }),
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        operation: OperationSpec::ConnectorLegacy {
            connector_name: "http-strict-pass".to_owned(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            payload: json!({"query":"ping"}),
        },
    };

    let report = execute_spec(spec, true).await;
    server.join().expect("join local http server");
    let runtime = &report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"];

    assert_eq!(report.operation_kind, "connector_legacy");
    assert_eq!(
        report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
        "executed"
    );
    assert_eq!(runtime["response_method"], "tools/call");
    assert_eq!(runtime["response_id"], expected_id);
    assert_http_json_runtime_keys(
        runtime,
        &[
            "request",
            "response_id",
            "response_json",
            "response_method",
            "response_text",
            "status_code",
        ],
    );
}
