use super::*;

#[tokio::test]
async fn execute_spec_programmatic_tool_call_supports_templates_and_steps() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-alpha".to_owned(),
            max_calls: 3,
            include_intermediate: true,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("notify".to_owned()),
            steps: vec![
                ProgrammaticStep::SetLiteral {
                    step_id: "seed".to_owned(),
                    value: json!({
                        "channel":"ops-alerts",
                        "message":"loongclaw-online"
                    }),
                },
                ProgrammaticStep::JsonPointer {
                    step_id: "msg".to_owned(),
                    from_step: "seed".to_owned(),
                    pointer: "/message".to_owned(),
                },
                ProgrammaticStep::ConnectorCall {
                    step_id: "notify".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({
                        "channel":"{{seed#/channel}}",
                        "text":"{{msg}}"
                    }),
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["connector_calls"], 1);
    assert_eq!(
        report.outcome["result"]["outcome"]["payload"]["payload"]["text"],
        "loongclaw-online"
    );
    assert_eq!(
        report.outcome["result"]["outcome"]["payload"]["payload"]["_loongclaw"]["caller"],
        "planner-alpha"
    );
    assert_eq!(
        report.outcome["step_outputs"]["msg"],
        Value::String("loongclaw-online".to_owned())
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_enforces_max_call_budget() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-budget".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-budget".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-beta".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: None,
            steps: vec![
                ProgrammaticStep::ConnectorCall {
                    step_id: "notify-a".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({"channel":"a"}),
                },
                ProgrammaticStep::ConnectorCall {
                    step_id: "notify-b".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({"channel":"b"}),
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("budget exceeded")
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_blocks_when_caller_not_allowlisted() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let plugin_root = std::env::temp_dir().join(format!("loongclaw-caller-acl-{unique}"));
    fs::create_dir_all(&plugin_root).expect("create plugin root");

    fs::write(
        plugin_root.join("acl_plugin.py"),
        r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "acl-guard",
#   "provider_id": "acl-provider",
#   "connector_name": "acl-provider",
#   "channel_id": "primary",
#   "endpoint": "https://example.com/acl",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"http_json",
#     "version":"1.0.0",
#     "allowed_callers":"planner-whitelisted"
#   }
# }
# LOONGCLAW_PLUGIN_END
"#,
    )
    .expect("write acl plugin");

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-caller-acl".to_owned(),
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
        agent_id: "agent-caller-acl".to_owned(),
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
        bootstrap: None,
        auto_provision: None,
        hotfixes: Vec::new(),
        plugin_setup_readiness: None,
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-denied".to_owned(),
            max_calls: 2,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["acl-provider".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "acl-call".to_owned(),
                connector_name: "acl-provider".to_owned(),
                operation: "invoke".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: None,
                priority_class: default_programmatic_priority_class(),
                payload: json!({"query":"hello"}),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("is not allowed")
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_connector_batch_parallel_succeeds() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-batch".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-batch".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-batch".to_owned(),
            max_calls: 4,
            include_intermediate: true,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("batch".to_owned()),
            steps: vec![
                ProgrammaticStep::SetLiteral {
                    step_id: "seed".to_owned(),
                    value: json!({"message":"batched"}),
                },
                ProgrammaticStep::ConnectorBatch {
                    step_id: "batch".to_owned(),
                    parallel: true,
                    continue_on_error: false,
                    calls: vec![
                        ProgrammaticBatchCall {
                            call_id: "a".to_owned(),
                            connector_name: "webhook".to_owned(),
                            operation: "notify".to_owned(),
                            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                            retry: None,
                            priority_class: default_programmatic_priority_class(),
                            payload: json!({"text":"{{seed#/message}}-a"}),
                        },
                        ProgrammaticBatchCall {
                            call_id: "b".to_owned(),
                            connector_name: "webhook".to_owned(),
                            operation: "notify".to_owned(),
                            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                            retry: None,
                            priority_class: default_programmatic_priority_class(),
                            payload: json!({"text":"{{seed#/message}}-b"}),
                        },
                    ],
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["connector_calls"], 2);
    assert_eq!(report.outcome["result"]["total_calls"], 2);
    assert_eq!(report.outcome["result"]["failed_calls"], 0);
    assert_eq!(
        report.outcome["result"]["by_call"]["a"]["outcome"]["payload"]["payload"]["_loongclaw"]["call_id"],
        "a"
    );
    assert_eq!(
        report.outcome["result"]["by_call"]["b"]["outcome"]["payload"]["payload"]["text"],
        "batched-b"
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_connector_batch_continue_on_error() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-batch-errors".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from([
                "webhook".to_owned(),
                "missing-connector".to_owned(),
            ]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-batch-errors".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-batch-errors".to_owned(),
            max_calls: 5,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from([
                "webhook".to_owned(),
                "missing-connector".to_owned(),
            ]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("batch".to_owned()),
            steps: vec![
                ProgrammaticStep::SetLiteral {
                    step_id: "seed".to_owned(),
                    value: json!({"message":"fanout"}),
                },
                ProgrammaticStep::ConnectorBatch {
                    step_id: "batch".to_owned(),
                    parallel: false,
                    continue_on_error: true,
                    calls: vec![
                        ProgrammaticBatchCall {
                            call_id: "ok-a".to_owned(),
                            connector_name: "webhook".to_owned(),
                            operation: "notify".to_owned(),
                            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                            retry: None,
                            priority_class: default_programmatic_priority_class(),
                            payload: json!({"text":"{{seed#/message}}-ok-a"}),
                        },
                        ProgrammaticBatchCall {
                            call_id: "bad".to_owned(),
                            connector_name: "missing-connector".to_owned(),
                            operation: "notify".to_owned(),
                            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                            retry: None,
                            priority_class: default_programmatic_priority_class(),
                            payload: json!({"text":"{{seed#/message}}-bad"}),
                        },
                        ProgrammaticBatchCall {
                            call_id: "ok-b".to_owned(),
                            connector_name: "webhook".to_owned(),
                            operation: "notify".to_owned(),
                            required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                            retry: None,
                            priority_class: default_programmatic_priority_class(),
                            payload: json!({"text":"{{seed#/message}}-ok-b"}),
                        },
                    ],
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["connector_calls"], 3);
    assert_eq!(report.outcome["result"]["failed_calls"], 1);
    assert_eq!(report.outcome["result"]["success_calls"], 2);
    assert_eq!(
        report.outcome["result"]["by_call"]["bad"]["status"],
        Value::String("error".to_owned())
    );
    assert!(
        report.outcome["result"]["by_call"]["bad"]["error"]
            .as_str()
            .expect("error string should exist")
            .contains("connector not found")
    );
    assert_eq!(
        report.outcome["result"]["by_call"]["bad"]["error_code"],
        Value::String("connector_not_found".to_owned())
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_conditional_step_routes_branch() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-conditional".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-conditional".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-conditional".to_owned(),
            max_calls: 2,
            include_intermediate: true,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("notify".to_owned()),
            steps: vec![
                ProgrammaticStep::SetLiteral {
                    step_id: "seed".to_owned(),
                    value: json!({
                        "channel":"ops-alerts",
                        "message":"conditional-ready",
                        "status":"green"
                    }),
                },
                ProgrammaticStep::Conditional {
                    step_id: "gate".to_owned(),
                    from_step: "seed".to_owned(),
                    pointer: Some("/status".to_owned()),
                    equals: Some(Value::String("green".to_owned())),
                    exists: Some(true),
                    when_true: json!({"text":"{{seed#/message}}"}),
                    when_false: Some(json!({"text":"blocked"})),
                },
                ProgrammaticStep::ConnectorCall {
                    step_id: "notify".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({
                        "channel":"{{seed#/channel}}",
                        "text":"{{gate#/text}}"
                    }),
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(
        report.outcome["step_outputs"]["gate"]["text"],
        "conditional-ready"
    );
    assert_eq!(
        report.outcome["result"]["outcome"]["payload"]["payload"]["text"],
        "conditional-ready"
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_connector_batch_budget_checks_total_calls() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-batch-budget".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-batch-budget".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-batch-budget".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: false,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"a"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"b"}),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert!(
        report
            .blocked_reason
            .as_deref()
            .expect("blocked reason should exist")
            .contains("budget exceeded")
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_retry_recovers_transient_failure() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let request_id = format!(
        "retry-success-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos()
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-retry".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-retry".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-retry".to_owned(),
            max_calls: 2,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("retry-call".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "retry-call".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: Some(ProgrammaticRetryPolicy {
                    max_attempts: 3,
                    initial_backoff_ms: 1,
                    max_backoff_ms: 10,
                    jitter_ratio: 0.0,
                    adaptive_jitter: false,
                }),
                priority_class: default_programmatic_priority_class(),
                payload: json!({
                    "channel":"ops-retry",
                    "_loongclaw_test": {
                        "request_id": request_id,
                        "failures_before_success": 1
                    }
                }),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["execution"]["attempts"], 2);
    assert_eq!(report.outcome["result"]["execution"]["retries"], 1);
    assert_eq!(
        report.outcome["result"]["outcome"]["status"],
        Value::String("ok".to_owned())
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_applies_connector_rate_limits() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-rate".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-rate".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-rate".to_owned(),
            max_calls: 2,
            include_intermediate: true,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::from([(
                "webhook".to_owned(),
                ProgrammaticConnectorRateLimit {
                    min_interval_ms: 15,
                },
            )]),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("call-b".to_owned()),
            steps: vec![
                ProgrammaticStep::ConnectorCall {
                    step_id: "call-a".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({"channel":"ops-a"}),
                },
                ProgrammaticStep::ConnectorCall {
                    step_id: "call-b".to_owned(),
                    connector_name: "webhook".to_owned(),
                    operation: "notify".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    retry: None,
                    priority_class: default_programmatic_priority_class(),
                    payload: json!({"channel":"ops-b"}),
                },
            ],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["connector_calls"], 2);
    let wait_ms = report.outcome["result"]["execution"]["rate_wait_ms_total"]
        .as_u64()
        .expect("rate wait should be number");
    assert!(
        wait_ms >= 1,
        "expected rate shaping to introduce wait on second call, got {wait_ms}"
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_rejects_invalid_rate_limit_policy() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-rate-invalid".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-rate-invalid".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-rate-invalid".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::from([(
                "webhook".to_owned(),
                ProgrammaticConnectorRateLimit { min_interval_ms: 0 },
            )]),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "call-a".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: None,
                priority_class: default_programmatic_priority_class(),
                payload: json!({"channel":"ops-a"}),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    let blocked_reason = report
        .blocked_reason
        .as_deref()
        .expect("blocked reason should exist");
    assert!(blocked_reason.contains("programmatic_error[invalid_spec]"));
    assert!(blocked_reason.contains("min_interval_ms"));
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_circuit_breaker_blocks_followup_batch_calls() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let request_id = format!(
        "circuit-open-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos()
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-circuit-open".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-circuit-open".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-circuit-open".to_owned(),
            max_calls: 2,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::from([(
                "webhook".to_owned(),
                ProgrammaticCircuitBreakerPolicy {
                    failure_threshold: 1,
                    cooldown_ms: 60_000,
                    half_open_max_calls: 1,
                    success_threshold: 1,
                },
            )]),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: false,
                continue_on_error: true,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "trip".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: Some(ProgrammaticRetryPolicy {
                            max_attempts: 1,
                            initial_backoff_ms: 1,
                            max_backoff_ms: 1,
                            jitter_ratio: 0.0,
                            adaptive_jitter: false,
                        }),
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-circuit-trip",
                            "_loongclaw_test": {
                                "request_id": request_id,
                                "failures_before_success": 9
                            }
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "followup".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-circuit-followup"
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["failed_calls"], 2);
    assert_eq!(
        report.outcome["result"]["by_call"]["followup"]["error_code"],
        Value::String("circuit_open".to_owned())
    );
    assert!(
        report.outcome["result"]["by_call"]["followup"]["error"]
            .as_str()
            .expect("followup error should exist")
            .contains("programmatic_error[circuit_open]")
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_rejects_invalid_circuit_policy() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-circuit-invalid".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-circuit-invalid".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-circuit-invalid".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::from([(
                "webhook".to_owned(),
                ProgrammaticCircuitBreakerPolicy {
                    failure_threshold: 0,
                    cooldown_ms: 10,
                    half_open_max_calls: 1,
                    success_threshold: 1,
                },
            )]),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "call-a".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: None,
                priority_class: default_programmatic_priority_class(),
                payload: json!({"channel":"ops-a"}),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    let blocked_reason = report
        .blocked_reason
        .as_deref()
        .expect("blocked reason should exist");
    assert!(blocked_reason.contains("programmatic_error[invalid_spec]"));
    assert!(blocked_reason.contains("failure_threshold"));
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_retry_jitter_tracks_backoff_budget() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let request_id = format!(
        "retry-jitter-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos()
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-retry-jitter".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-retry-jitter".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-retry-jitter".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: default_programmatic_concurrency_policy(),
            return_step: Some("call".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "call".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: Some(ProgrammaticRetryPolicy {
                    max_attempts: 3,
                    initial_backoff_ms: 5,
                    max_backoff_ms: 80,
                    jitter_ratio: 0.5,
                    adaptive_jitter: true,
                }),
                priority_class: default_programmatic_priority_class(),
                payload: json!({
                    "channel":"ops-retry-jitter",
                    "_loongclaw_test": {
                        "request_id": request_id,
                        "failures_before_success": 2
                    }
                }),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["execution"]["attempts"], 3);
    assert_eq!(report.outcome["result"]["execution"]["retries"], 2);
    let backoff_ms = report.outcome["result"]["execution"]["backoff_ms_total"]
        .as_u64()
        .expect("backoff budget should be numeric");
    assert!(
        backoff_ms >= 15,
        "expected jittered backoff to accumulate over retries, got {backoff_ms}"
    );
    assert_eq!(
        report.outcome["result"]["execution"]["circuit_phase_before"],
        Value::String("disabled".to_owned())
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_parallel_batch_caps_peak_inflight_budget() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-concurrency-cap".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-concurrency-cap".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-concurrency-cap".to_owned(),
            max_calls: 6,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 2,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: false,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 2,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: false,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-a",
                            "_loongclaw_test": {"delay_ms": 25}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-b",
                            "_loongclaw_test": {"delay_ms": 25}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "c".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-c",
                            "_loongclaw_test": {"delay_ms": 25}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "d".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-d",
                            "_loongclaw_test": {"delay_ms": 25}
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(
        report.outcome["result"]["scheduler"]["configured_max_in_flight"],
        2
    );
    assert_eq!(report.outcome["result"]["scheduler"]["peak_in_flight"], 2);
    assert_eq!(
        report.outcome["result"]["scheduler"]["mode"],
        Value::String("parallel".to_owned())
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_weighted_fairness_avoids_low_priority_starvation() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-fairness".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-fairness".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-fairness".to_owned(),
            max_calls: 8,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 1,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: false,
                high_weight: 3,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 2,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: false,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "high-1".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: ProgrammaticPriorityClass::High,
                        payload: json!({"channel":"ops-high-1"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "high-2".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: ProgrammaticPriorityClass::High,
                        payload: json!({"channel":"ops-high-2"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "high-3".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: ProgrammaticPriorityClass::High,
                        payload: json!({"channel":"ops-high-3"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "high-4".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: ProgrammaticPriorityClass::High,
                        payload: json!({"channel":"ops-high-4"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "low-1".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: ProgrammaticPriorityClass::Low,
                        payload: json!({"channel":"ops-low-1"}),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    let dispatch_order = report.outcome["result"]["scheduler"]["dispatch_order"]
        .as_array()
        .expect("dispatch order should be array");
    assert_eq!(dispatch_order.len(), 5);
    assert_eq!(dispatch_order[3], Value::String("low-1".to_owned()));
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_adaptive_budget_reduces_on_failures() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let request_id = format!(
        "adaptive-budget-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos()
    );

    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-adaptive-budget".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-adaptive-budget".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-adaptive-budget".to_owned(),
            max_calls: 8,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 3,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 10,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: true,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "trip".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: Some(ProgrammaticRetryPolicy {
                            max_attempts: 1,
                            initial_backoff_ms: 1,
                            max_backoff_ms: 1,
                            jitter_ratio: 0.0,
                            adaptive_jitter: false,
                        }),
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-adaptive-trip",
                            "_loongclaw_test": {
                                "request_id": request_id,
                                "failures_before_success": 9
                            }
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "slow-a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-adaptive-a",
                            "_loongclaw_test": {"delay_ms": 30}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "slow-b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-adaptive-b",
                            "_loongclaw_test": {"delay_ms": 30}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "slow-c".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-adaptive-c",
                            "_loongclaw_test": {"delay_ms": 30}
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["failed_calls"], 1);
    assert_eq!(report.outcome["result"]["scheduler"]["peak_in_flight"], 3);
    let reductions = report.outcome["result"]["scheduler"]["budget_reductions"]
        .as_u64()
        .expect("budget_reductions should be numeric");
    assert!(reductions >= 1, "expected adaptive budget reduction");
    assert_eq!(
        report.outcome["result"]["scheduler"]["final_in_flight_budget"],
        2
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_default_adaptive_policy_skips_not_found_errors() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-adaptive-skip-not-found".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-adaptive-skip-not-found".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-adaptive-skip-not-found".to_owned(),
            max_calls: 6,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 3,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 99,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: true,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "ok-a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-a",
                            "_loongclaw_test":{"delay_ms":20}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "bad-missing".to_owned(),
                        connector_name: "missing".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"ops-bad"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-b",
                            "_loongclaw_test":{"delay_ms":20}
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["failed_calls"], 1);
    assert_eq!(
        report.outcome["result"]["by_call"]["bad-missing"]["error_code"],
        Value::String("connector_not_found".to_owned())
    );
    assert_eq!(
        report.outcome["result"]["scheduler"]["budget_reductions"],
        0
    );
    assert_eq!(
        report.outcome["result"]["scheduler"]["final_in_flight_budget"],
        3
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_adaptive_policy_can_reduce_on_any_error() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-adaptive-any-error".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-adaptive-any-error".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-adaptive-any-error".to_owned(),
            max_calls: 6,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 4,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 99,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 2,
                adaptive_reduce_on: BTreeSet::from([ProgrammaticAdaptiveReduceOn::AnyError]),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: true,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "ok-a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-a",
                            "_loongclaw_test":{"delay_ms":20}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "bad-missing".to_owned(),
                        connector_name: "missing".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"ops-bad"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-b",
                            "_loongclaw_test":{"delay_ms":20}
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["failed_calls"], 1);
    assert_eq!(
        report.outcome["result"]["by_call"]["bad-missing"]["error_code"],
        Value::String("connector_not_found".to_owned())
    );
    let reductions = report.outcome["result"]["scheduler"]["budget_reductions"]
        .as_u64()
        .expect("budget_reductions should be numeric");
    assert!(
        reductions >= 1,
        "expected adaptive budget reduction on any_error"
    );
    assert_eq!(
        report.outcome["result"]["scheduler"]["final_in_flight_budget"],
        2
    );
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_rejects_invalid_concurrency_policy() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-concurrency-invalid".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-concurrency-invalid".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-concurrency-invalid".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 0,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 1,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
            },
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "call".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: None,
                priority_class: default_programmatic_priority_class(),
                payload: json!({"channel":"ops-invalid"}),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    let blocked_reason = report
        .blocked_reason
        .as_deref()
        .expect("blocked reason should exist");
    assert!(blocked_reason.contains("programmatic_error[invalid_spec]"));
    assert!(blocked_reason.contains("max_in_flight"));
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_rejects_empty_adaptive_reduce_on_policy() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-concurrency-empty-reduce-on".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-concurrency-empty-reduce-on".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-concurrency-empty-reduce-on".to_owned(),
            max_calls: 1,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 2,
                min_in_flight: 1,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 1,
                adaptive_upshift_step: 1,
                adaptive_downshift_step: 1,
                adaptive_reduce_on: BTreeSet::new(),
            },
            return_step: None,
            steps: vec![ProgrammaticStep::ConnectorCall {
                step_id: "call".to_owned(),
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                retry: None,
                priority_class: default_programmatic_priority_class(),
                payload: json!({"channel":"ops-invalid-empty-reduce"}),
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "blocked",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    let blocked_reason = report
        .blocked_reason
        .as_deref()
        .expect("blocked reason should exist");
    assert!(blocked_reason.contains("programmatic_error[invalid_spec]"));
    assert!(blocked_reason.contains("adaptive_reduce_on"));
}

#[tokio::test]
async fn programmatic_circuit_runtime_transitions_open_half_open_and_closes_on_success() {
    let policies = BTreeMap::from([(
        "webhook".to_owned(),
        ProgrammaticCircuitBreakerPolicy {
            failure_threshold: 1,
            cooldown_ms: 15,
            half_open_max_calls: 1,
            success_threshold: 1,
        },
    )]);
    let state = Arc::new(tokio::sync::Mutex::new(BTreeMap::<
        String,
        ProgrammaticCircuitRuntimeState,
    >::new()));

    let initial = acquire_programmatic_circuit_slot(
        "webhook",
        &policies,
        &state,
        "test-step",
        Some("initial"),
    )
    .await
    .expect("initial closed slot should be granted");
    assert_eq!(initial, "closed");

    let after_failure =
        record_programmatic_circuit_outcome("webhook", false, &policies, &state).await;
    assert_eq!(after_failure, "open");

    let blocked = acquire_programmatic_circuit_slot(
        "webhook",
        &policies,
        &state,
        "test-step",
        Some("blocked"),
    )
    .await
    .expect_err("circuit should block while open");
    assert!(blocked.contains("programmatic_error[circuit_open]"));

    sleep(Duration::from_millis(20)).await;

    let half_open = acquire_programmatic_circuit_slot(
        "webhook",
        &policies,
        &state,
        "test-step",
        Some("half-open"),
    )
    .await
    .expect("slot should reopen in half-open after cooldown");
    assert_eq!(half_open, "half_open");

    let after_success =
        record_programmatic_circuit_outcome("webhook", true, &policies, &state).await;
    assert_eq!(after_success, "closed");

    let closed_again = acquire_programmatic_circuit_slot(
        "webhook",
        &policies,
        &state,
        "test-step",
        Some("closed-again"),
    )
    .await
    .expect("circuit should close again after successful half-open probe");
    assert_eq!(closed_again, "closed");
}

#[tokio::test]
async fn execute_spec_programmatic_tool_call_adaptive_budget_respects_min_floor_and_recovers() {
    let spec = RunnerSpec {
        pack: VerticalPackManifest {
            pack_id: "spec-programmatic-adaptive-floor-recover".to_owned(),
            domain: "ops".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            granted_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            metadata: BTreeMap::new(),
        },
        agent_id: "agent-programmatic-adaptive-floor-recover".to_owned(),
        ttl_s: 120,
        approval: Some(HumanApprovalSpec {
            mode: HumanApprovalMode::Disabled,
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
        operation: OperationSpec::ProgrammaticToolCall {
            caller: "planner-adaptive-floor-recover".to_owned(),
            max_calls: 8,
            include_intermediate: false,
            allowed_connectors: BTreeSet::from(["webhook".to_owned(), "missing".to_owned()]),
            connector_rate_limits: BTreeMap::new(),
            connector_circuit_breakers: BTreeMap::new(),
            concurrency: ProgrammaticConcurrencyPolicy {
                max_in_flight: 5,
                min_in_flight: 2,
                fairness: ProgrammaticFairnessPolicy::WeightedRoundRobin,
                adaptive_budget: true,
                high_weight: 1,
                normal_weight: 1,
                low_weight: 1,
                adaptive_recovery_successes: 1,
                adaptive_upshift_step: 2,
                adaptive_downshift_step: 3,
                adaptive_reduce_on: BTreeSet::from([ProgrammaticAdaptiveReduceOn::AnyError]),
            },
            return_step: Some("batch".to_owned()),
            steps: vec![ProgrammaticStep::ConnectorBatch {
                step_id: "batch".to_owned(),
                parallel: true,
                continue_on_error: true,
                calls: vec![
                    ProgrammaticBatchCall {
                        call_id: "bad-a".to_owned(),
                        connector_name: "missing".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"ops-bad-a"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "bad-b".to_owned(),
                        connector_name: "missing".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({"channel":"ops-bad-b"}),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-a".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-a",
                            "_loongclaw_test": {"delay_ms": 20}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-b".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-b",
                            "_loongclaw_test": {"delay_ms": 20}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-c".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-c",
                            "_loongclaw_test": {"delay_ms": 20}
                        }),
                    },
                    ProgrammaticBatchCall {
                        call_id: "ok-d".to_owned(),
                        connector_name: "webhook".to_owned(),
                        operation: "notify".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        retry: None,
                        priority_class: default_programmatic_priority_class(),
                        payload: json!({
                            "channel":"ops-ok-d",
                            "_loongclaw_test": {"delay_ms": 20}
                        }),
                    },
                ],
            }],
        },
    };

    let report = execute_spec(&spec, true).await;
    assert_eq!(
        report.operation_kind, "programmatic_tool_call",
        "blocked_reason={:?}, outcome={}",
        report.blocked_reason, report.outcome
    );
    assert_eq!(report.outcome["result"]["failed_calls"], 2);
    assert_eq!(
        report.outcome["result"]["scheduler"]["configured_min_in_flight"],
        2
    );
    let reductions = report.outcome["result"]["scheduler"]["budget_reductions"]
        .as_u64()
        .expect("budget_reductions should be numeric");
    let increases = report.outcome["result"]["scheduler"]["budget_increases"]
        .as_u64()
        .expect("budget_increases should be numeric");
    assert!(reductions >= 1, "expected at least one budget reduction");
    assert!(
        increases >= 1,
        "expected recovery increases after successes"
    );
    assert_eq!(
        report.outcome["result"]["scheduler"]["final_in_flight_budget"],
        5
    );
}
