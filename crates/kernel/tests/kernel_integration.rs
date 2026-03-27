use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use loongclaw_kernel::test_support::*;
use loongclaw_kernel::*;
use serde_json::json;

#[tokio::test]
async fn integration_kernel_executes_task() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel.register_pack(sample_pack()).unwrap();
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .unwrap();
    let task = TaskIntent {
        task_id: "t-1".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let dispatch = kernel
        .execute_task("sales-intel", &token, task)
        .await
        .unwrap();
    assert_eq!(dispatch.outcome.status, "ok");
}

#[tokio::test]
async fn kernel_executes_task_and_connector_under_pack_policy() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_core_connector_adapter(MockCrmConnector);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let task = TaskIntent {
        task_id: "task-001".to_owned(),
        objective: "summarize top accounts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
        payload: json!({"accounts": ["acme", "globex"]}),
    };

    let dispatch = kernel
        .execute_task("sales-intel", &token, task)
        .await
        .expect("task should dispatch");

    assert_eq!(dispatch.adapter_route.harness_kind, HarnessKind::EmbeddedPi);
    assert_eq!(dispatch.outcome.status, "ok");

    let connector_command = ConnectorCommand {
        connector_name: "crm".to_owned(),
        operation: "upsert_lead".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"lead_id": "L-1000"}),
    };

    let connector_dispatch = kernel
        .execute_connector_core("sales-intel", &token, None, connector_command)
        .await
        .expect("connector dispatch should succeed");

    assert_eq!(connector_dispatch.connector_name, "crm");
    assert_eq!(connector_dispatch.outcome.status, "ok");
}

#[tokio::test]
async fn kernel_rejects_token_missing_capability() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });

    let mut token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");
    token.allowed_capabilities.remove(&Capability::MemoryRead);

    let task = TaskIntent {
        task_id: "task-002".to_owned(),
        objective: "read account memory".to_owned(),
        required_capabilities: BTreeSet::from([Capability::MemoryRead]),
        payload: json!({}),
    };

    let error = kernel
        .execute_task("sales-intel", &token, task)
        .await
        .expect_err("missing capability should fail");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::MissingCapability { .. })
    ));
}

#[tokio::test]
async fn kernel_rejects_connector_not_whitelisted_by_pack() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_core_connector_adapter(MockCrmConnector);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "erp".to_owned(),
        operation: "sync".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({}),
    };

    let error = kernel
        .execute_connector_core("sales-intel", &token, None, command)
        .await
        .expect_err("non-whitelisted connector must fail");

    assert!(matches!(
        error,
        KernelError::ConnectorNotAllowed {
            connector,
            pack_id
        } if connector == "erp" && pack_id == "sales-intel"
    ));
}

#[tokio::test]
async fn layered_connector_core_executes_through_core_plane() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_core_connector_adapter(MockCoreConnector);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "crm".to_owned(),
        operation: "sync_contacts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"batch": 3}),
    };

    let dispatch = kernel
        .execute_connector_core("sales-intel", &token, None, command)
        .await
        .expect("core connector plane should execute");

    assert_eq!(dispatch.connector_name, "crm");
    assert_eq!(dispatch.outcome.payload["tier"], "core");
    assert_eq!(dispatch.outcome.payload["adapter"], "http-core");
}

#[tokio::test]
async fn layered_connector_extension_composes_over_core_plane() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_core_connector_adapter(MockCoreConnector);
    kernel.register_connector_extension_adapter(MockConnectorExtension);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "crm".to_owned(),
        operation: "upsert_accounts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"source": "erp"}),
    };

    let dispatch = kernel
        .execute_connector_extension("sales-intel", &token, "shielded-bridge", None, command)
        .await
        .expect("extension connector plane should execute");

    assert_eq!(dispatch.connector_name, "crm");
    assert_eq!(dispatch.outcome.payload["tier"], "extension");
    assert_eq!(dispatch.outcome.payload["extension"], "shielded-bridge");
    assert_eq!(dispatch.outcome.payload["core_probe"]["tier"], "core");
}

#[tokio::test]
async fn layered_connector_plane_still_enforces_pack_whitelist() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_core_connector_adapter(MockCoreConnector);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "erp".to_owned(),
        operation: "sync_accounts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({}),
    };

    let error = kernel
        .execute_connector_core("sales-intel", &token, None, command)
        .await
        .expect_err("connector outside whitelist should be denied");

    assert!(matches!(
        error,
        KernelError::ConnectorNotAllowed {
            connector,
            pack_id
        } if connector == "erp" && pack_id == "sales-intel"
    ));
}

#[tokio::test]
async fn layered_connector_extension_requires_available_core_adapter() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_connector_extension_adapter(MockConnectorExtension);

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "crm".to_owned(),
        operation: "upsert_accounts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({}),
    };

    let error = kernel
        .execute_connector_extension("sales-intel", &token, "shielded-bridge", None, command)
        .await
        .expect_err("extension path must fail without core adapter");

    assert!(matches!(
        error,
        KernelError::Connector(ConnectorError::NoDefaultCoreAdapter)
    ));
}

#[tokio::test]
async fn layered_connector_default_core_adapter_can_be_overridden() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_core_connector_adapter(MockCoreConnector);
    kernel.register_core_connector_adapter(MockCoreConnectorGrpc);
    kernel
        .set_default_core_connector_adapter("grpc-core")
        .expect("default adapter should be set");

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let command = ConnectorCommand {
        connector_name: "crm".to_owned(),
        operation: "sync_contacts".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
        payload: json!({"batch": 1}),
    };

    let dispatch = kernel
        .execute_connector_core("sales-intel", &token, None, command)
        .await
        .expect("core connector plane should execute");

    assert_eq!(dispatch.outcome.payload["adapter"], "grpc-core");
}

#[test]
fn layered_connector_rejects_unknown_default_adapter_override() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel.register_core_connector_adapter(MockCoreConnector);

    let error = kernel
        .set_default_core_connector_adapter("missing-core")
        .expect_err("missing adapter should fail");

    assert!(matches!(
        error,
        KernelError::Connector(ConnectorError::CoreAdapterNotFound(name)) if name == "missing-core"
    ));
}

#[tokio::test]
async fn kernel_auto_routes_by_harness_kind_when_adapter_is_not_pinned() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(acp_pack_without_explicit_adapter())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_harness_adapter(MockAcpHarness);

    let token = kernel
        .issue_token("code-review", "agent-coder", 120)
        .expect("token should issue");

    let task = TaskIntent {
        task_id: "task-acp-01".to_owned(),
        objective: "review the pull request".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };

    let dispatch = kernel
        .execute_task("code-review", &token, task)
        .await
        .expect("dispatch should succeed");

    assert_eq!(dispatch.adapter_route.harness_kind, HarnessKind::Acp);
    assert_eq!(dispatch.outcome.output["adapter"], "acp-gateway");
}

#[tokio::test]
async fn revoked_token_is_denied_by_policy_engine() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");
    kernel
        .revoke_token(&token.token_id, Some("agent-alpha"))
        .expect("revoke should succeed");

    let task = TaskIntent {
        task_id: "task-003".to_owned(),
        objective: "try a revoked token".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };

    let error = kernel
        .execute_task("sales-intel", &token, task)
        .await
        .expect_err("revoked token should fail");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::RevokedToken { token_id }) if token_id == token.token_id
    ));
}

#[tokio::test]
async fn audit_sink_receives_core_lifecycle_events() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());

    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_core_connector_adapter(MockCrmConnector);

    let token = kernel
        .issue_token("sales-intel", "agent-audit", 300)
        .expect("token should issue");

    let task = TaskIntent {
        task_id: "task-audit-01".to_owned(),
        objective: "audit me".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    kernel
        .execute_task("sales-intel", &token, task)
        .await
        .expect("task should dispatch");

    kernel
        .execute_connector_core(
            "sales-intel",
            &token,
            None,
            ConnectorCommand {
                connector_name: "crm".to_owned(),
                operation: "upsert".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"ok": true}),
            },
        )
        .await
        .expect("connector call should succeed");

    clock.advance_by(10);
    kernel
        .revoke_token(&token.token_id, Some("agent-audit"))
        .expect("revoke should succeed");

    let snapshot = audit.snapshot();
    assert!(snapshot.len() >= 4);
    assert!(
        snapshot
            .iter()
            .any(|event| { matches!(event.kind, AuditEventKind::TokenIssued { .. }) })
    );
    assert!(
        snapshot
            .iter()
            .any(|event| { matches!(event.kind, AuditEventKind::TaskDispatched { .. }) })
    );
    assert!(
        snapshot
            .iter()
            .any(|event| { matches!(event.kind, AuditEventKind::ConnectorInvoked { .. }) })
    );
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Connector,
                tier: PlaneTier::Core,
                ..
            }
        )
    }));
    assert!(snapshot.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::TokenRevoked { token_id } if token_id == &token.token_id
        )
    }));
}

#[test]
fn record_audit_event_supports_security_scan_summary() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_123));
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    kernel
        .record_audit_event(
            Some("agent-security"),
            AuditEventKind::SecurityScanEvaluated {
                pack_id: "pack-security".to_owned(),
                scanned_plugins: 2,
                total_findings: 3,
                high_findings: 1,
                medium_findings: 1,
                low_findings: 1,
                blocked: true,
                block_reason: Some("security scan blocked 1 high-risk finding(s)".to_owned()),
                categories: vec![
                    "process_command_not_allowlisted".to_owned(),
                    "wasm_import_prefix_blocked".to_owned(),
                ],
                finding_ids: vec![
                    "sf-1111111111111111".to_owned(),
                    "sf-2222222222222222".to_owned(),
                ],
            },
        )
        .expect("custom audit event should record");

    let events = audit.snapshot();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "evt-0000000000000001");
    assert_eq!(events[0].timestamp_epoch_s, 1_700_000_123);
    assert_eq!(events[0].agent_id.as_deref(), Some("agent-security"));
    assert!(matches!(
        &events[0].kind,
        AuditEventKind::SecurityScanEvaluated {
            pack_id,
            scanned_plugins,
            blocked,
            ..
        } if pack_id == "pack-security" && *scanned_plugins == 2 && *blocked
    ));
}

#[test]
fn record_audit_event_supports_provider_failover_summary() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_123));
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    kernel
        .record_audit_event(
            Some("agent-provider"),
            AuditEventKind::ProviderFailover {
                pack_id: "pack-provider".to_owned(),
                provider_id: "openai".to_owned(),
                reason: "rate_limited".to_owned(),
                stage: "status_failure".to_owned(),
                model: "gpt-5".to_owned(),
                attempt: 2,
                max_attempts: 3,
                status_code: Some(429),
                try_next_model: true,
                auto_model_mode: true,
                candidate_index: 1,
                candidate_count: 4,
            },
        )
        .expect("provider failover audit event should record");

    let events = audit.snapshot();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "evt-0000000000000001");
    assert_eq!(events[0].timestamp_epoch_s, 1_700_000_123);
    assert_eq!(events[0].agent_id.as_deref(), Some("agent-provider"));
    assert!(matches!(
        &events[0].kind,
        AuditEventKind::ProviderFailover {
            pack_id,
            provider_id,
            reason,
            stage,
            model,
            attempt,
            max_attempts,
            status_code,
            try_next_model,
            auto_model_mode,
            candidate_index,
            candidate_count,
        } if pack_id == "pack-provider"
            && provider_id == "openai"
            && reason == "rate_limited"
            && stage == "status_failure"
            && model == "gpt-5"
            && *attempt == 2
            && *max_attempts == 3
            && *status_code == Some(429)
            && *try_next_model
            && *auto_model_mode
            && *candidate_index == 1
            && *candidate_count == 4
    ));
}

#[test]
fn record_audit_event_supports_plugin_trust_summary() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_124));
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    kernel
        .record_audit_event(
            Some("agent-plugin-trust"),
            AuditEventKind::PluginTrustEvaluated {
                pack_id: "pack-plugin-trust".to_owned(),
                scanned_plugins: 3,
                official_plugins: 1,
                verified_community_plugins: 1,
                unverified_plugins: 1,
                high_risk_plugins: 2,
                high_risk_unverified_plugins: 1,
                blocked_auto_apply_plugins: 1,
                review_required_plugin_ids: vec!["stdio-review".to_owned()],
                review_required_bridges: vec!["process_stdio".to_owned()],
            },
        )
        .expect("plugin trust audit event should record");

    let events = audit.snapshot();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "evt-0000000000000001");
    assert_eq!(events[0].timestamp_epoch_s, 1_700_000_124);
    assert_eq!(events[0].agent_id.as_deref(), Some("agent-plugin-trust"));
    assert!(matches!(
        &events[0].kind,
        AuditEventKind::PluginTrustEvaluated {
            pack_id,
            scanned_plugins,
            official_plugins,
            verified_community_plugins,
            unverified_plugins,
            high_risk_plugins,
            high_risk_unverified_plugins,
            blocked_auto_apply_plugins,
            review_required_plugin_ids,
            review_required_bridges,
        } if pack_id == "pack-plugin-trust"
            && *scanned_plugins == 3
            && *official_plugins == 1
            && *verified_community_plugins == 1
            && *unverified_plugins == 1
            && *high_risk_plugins == 2
            && *high_risk_unverified_plugins == 1
            && *blocked_auto_apply_plugins == 1
            && review_required_plugin_ids == &vec!["stdio-review".to_owned()]
            && review_required_bridges == &vec!["process_stdio".to_owned()]
    ));
}

#[test]
fn record_audit_event_supports_tool_search_summary() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_125));
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

    kernel
        .record_audit_event(
            Some("agent-tool-search"),
            AuditEventKind::ToolSearchEvaluated {
                pack_id: "pack-tool-search".to_owned(),
                query: "trust:official search".to_owned(),
                returned: 0,
                trust_filter_applied: true,
                query_requested_tiers: vec!["official".to_owned()],
                structured_requested_tiers: vec!["verified-community".to_owned()],
                effective_tiers: Vec::new(),
                conflicting_requested_tiers: true,
                filtered_out_candidates: 2,
                filtered_out_tier_counts: BTreeMap::from([
                    ("official".to_owned(), 1_usize),
                    ("verified-community".to_owned(), 1_usize),
                ]),
                top_provider_ids: Vec::new(),
            },
        )
        .expect("tool search audit event should record");

    let events = audit.snapshot();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "evt-0000000000000001");
    assert_eq!(events[0].timestamp_epoch_s, 1_700_000_125);
    assert_eq!(events[0].agent_id.as_deref(), Some("agent-tool-search"));
    assert!(matches!(
        &events[0].kind,
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
        } if pack_id == "pack-tool-search"
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
    ));
}

#[tokio::test]
async fn layered_runtime_tool_and_memory_paths_execute_via_core_and_extension() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "layered-dev".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::MemoryRead,
                Capability::MemoryWrite,
                Capability::ObserveTelemetry,
            ]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_core_runtime_adapter(MockCoreRuntime);
    kernel.register_runtime_extension_adapter(MockRuntimeExtension);
    kernel.register_core_tool_adapter(MockCoreTool);
    kernel.register_tool_extension_adapter(MockToolExtension);
    kernel.register_core_memory_adapter(MockCoreMemory);
    kernel.register_memory_extension_adapter(MockMemoryExtension);

    let token = kernel
        .issue_token("layered-dev", "agent-layered", 120)
        .expect("token should issue");

    let runtime_required = BTreeSet::from([Capability::ObserveTelemetry]);
    let runtime_outcome = kernel
        .execute_runtime_extension(
            "layered-dev",
            &token,
            &runtime_required,
            "acp-bridge",
            None,
            RuntimeExtensionRequest {
                action: "start-session".to_owned(),
                payload: json!({"session": "s-1"}),
            },
        )
        .await
        .expect("runtime extension should execute");
    assert_eq!(runtime_outcome.status, "ok");
    assert_eq!(runtime_outcome.payload["extension"], "acp-bridge");

    let tool_required = BTreeSet::from([Capability::InvokeTool]);
    let tool_outcome = kernel
        .execute_tool_extension(
            "layered-dev",
            &token,
            &tool_required,
            "sql-analytics",
            None,
            ToolExtensionRequest {
                extension_action: "aggregate".to_owned(),
                payload: json!({"metric": "revenue"}),
            },
        )
        .await
        .expect("tool extension should execute");
    assert_eq!(tool_outcome.status, "ok");
    assert_eq!(tool_outcome.payload["extension"], "sql-analytics");

    let memory_required = BTreeSet::from([Capability::MemoryRead]);
    let memory_outcome = kernel
        .execute_memory_extension(
            "layered-dev",
            &token,
            &memory_required,
            "vector-index",
            None,
            MemoryExtensionRequest {
                operation: "semantic_search".to_owned(),
                payload: json!({"query": "top customer"}),
            },
        )
        .await
        .expect("memory extension should execute");
    assert_eq!(memory_outcome.status, "ok");
    assert_eq!(memory_outcome.payload["extension"], "vector-index");
}

#[tokio::test]
async fn audit_sink_captures_runtime_tool_memory_and_connector_plane_events() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_100));
    let audit = Arc::new(InMemoryAuditSink::default());

    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "audit-layered".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["crm".to_owned()]),
            granted_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::InvokeConnector,
                Capability::MemoryRead,
                Capability::ObserveTelemetry,
            ]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_core_connector_adapter(MockCoreConnector);
    kernel.register_connector_extension_adapter(MockConnectorExtension);
    kernel.register_core_runtime_adapter(MockCoreRuntime);
    kernel.register_runtime_extension_adapter(MockRuntimeExtension);
    kernel.register_core_tool_adapter(MockCoreTool);
    kernel.register_tool_extension_adapter(MockToolExtension);
    kernel.register_core_memory_adapter(MockCoreMemory);
    kernel.register_memory_extension_adapter(MockMemoryExtension);

    let token = kernel
        .issue_token("audit-layered", "agent-audit-plane", 120)
        .expect("token should issue");

    kernel
        .execute_connector_core(
            "audit-layered",
            &token,
            None,
            ConnectorCommand {
                connector_name: "crm".to_owned(),
                operation: "core_sync".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        )
        .await
        .expect("connector core should execute");
    kernel
        .execute_connector_extension(
            "audit-layered",
            &token,
            "shielded-bridge",
            None,
            ConnectorCommand {
                connector_name: "crm".to_owned(),
                operation: "ext_sync".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        )
        .await
        .expect("connector extension should execute");
    kernel
        .execute_runtime_extension(
            "audit-layered",
            &token,
            &BTreeSet::from([Capability::ObserveTelemetry]),
            "acp-bridge",
            None,
            RuntimeExtensionRequest {
                action: "start".to_owned(),
                payload: json!({}),
            },
        )
        .await
        .expect("runtime extension should execute");
    kernel
        .execute_tool_extension(
            "audit-layered",
            &token,
            &BTreeSet::from([Capability::InvokeTool]),
            "sql-analytics",
            None,
            ToolExtensionRequest {
                extension_action: "aggregate".to_owned(),
                payload: json!({}),
            },
        )
        .await
        .expect("tool extension should execute");
    kernel
        .execute_memory_extension(
            "audit-layered",
            &token,
            &BTreeSet::from([Capability::MemoryRead]),
            "vector-index",
            None,
            MemoryExtensionRequest {
                operation: "semantic_search".to_owned(),
                payload: json!({}),
            },
        )
        .await
        .expect("memory extension should execute");

    let snapshot = audit.snapshot();
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Connector,
                tier: PlaneTier::Core,
                ..
            }
        )
    }));
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Connector,
                tier: PlaneTier::Extension,
                ..
            }
        )
    }));
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Extension,
                ..
            }
        )
    }));
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Tool,
                tier: PlaneTier::Extension,
                ..
            }
        )
    }));
    assert!(snapshot.iter().any(|event| {
        matches!(
            event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Memory,
                tier: PlaneTier::Extension,
                ..
            }
        )
    }));
}

#[tokio::test]
async fn policy_extension_chain_can_block_high_risk_capabilities() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "strict-env".to_owned(),
            domain: "security".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::NetworkEgress,
            ]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_policy_extension(NoNetworkEgressPolicyExtension);

    let token = kernel
        .issue_token("strict-env", "agent-secure", 120)
        .expect("token should issue");

    let risky_task = TaskIntent {
        task_id: "task-net-01".to_owned(),
        objective: "fetch external url".to_owned(),
        required_capabilities: BTreeSet::from([Capability::NetworkEgress]),
        payload: json!({"url": "https://example.com"}),
    };

    let error = kernel
        .execute_task("strict-env", &token, risky_task)
        .await
        .expect_err("policy extension should block network egress");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::ExtensionDenied { extension, .. }) if extension == "no-network-egress"
    ));
}

#[tokio::test]
async fn plane_audit_records_resolved_default_core_adapter_names() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_000_200));
    let audit = Arc::new(InMemoryAuditSink::default());

    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "audit-defaults".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::from(["crm".to_owned()]),
            granted_capabilities: BTreeSet::from([
                Capability::InvokeConnector,
                Capability::ObserveTelemetry,
            ]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_core_connector_adapter(MockCoreConnector);
    kernel.register_core_connector_adapter(MockCoreConnectorGrpc);
    kernel
        .set_default_core_connector_adapter("grpc-core")
        .expect("default connector core should be set");
    kernel.register_connector_extension_adapter(MockConnectorExtension);

    kernel.register_core_runtime_adapter(MockCoreRuntime);
    kernel.register_core_runtime_adapter(MockCoreRuntimeFallback);
    kernel
        .set_default_core_runtime_adapter("fallback-core")
        .expect("default runtime core should be set");

    let token = kernel
        .issue_token("audit-defaults", "agent-default-audit", 120)
        .expect("token should issue");

    kernel
        .execute_connector_extension(
            "audit-defaults",
            &token,
            "shielded-bridge",
            None,
            ConnectorCommand {
                connector_name: "crm".to_owned(),
                operation: "sync".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        )
        .await
        .expect("connector extension should execute");

    kernel
        .execute_runtime_core(
            "audit-defaults",
            &token,
            &BTreeSet::from([Capability::ObserveTelemetry]),
            None,
            RuntimeCoreRequest {
                action: "boot".to_owned(),
                payload: json!({}),
            },
        )
        .await
        .expect("runtime core should execute");

    let snapshot = audit.snapshot();

    #[allow(clippy::wildcard_enum_match_arm)]
    let connector_ext_event = snapshot
        .iter()
        .find_map(|event| match &event.kind {
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Connector,
                tier: PlaneTier::Extension,
                primary_adapter,
                delegated_core_adapter,
                ..
            } => Some((primary_adapter.clone(), delegated_core_adapter.clone())),
            _ => None,
        })
        .expect("connector extension plane event should exist");
    assert_eq!(connector_ext_event.0, "shielded-bridge");
    assert_eq!(connector_ext_event.1.as_deref(), Some("grpc-core"));

    #[allow(clippy::wildcard_enum_match_arm)]
    let runtime_core_event = snapshot
        .iter()
        .find_map(|event| match &event.kind {
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter,
                delegated_core_adapter,
                ..
            } => Some((primary_adapter.clone(), delegated_core_adapter.clone())),
            _ => None,
        })
        .expect("runtime core plane event should exist");
    assert_eq!(runtime_core_event.0, "fallback-core");
    assert_eq!(runtime_core_event.1, None);
}

#[tokio::test]
async fn audit_event_json_schema_for_plane_invoked_is_stable() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_001_000));
    let audit = Arc::new(InMemoryAuditSink::default());

    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "audit-schema".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_core_runtime_adapter(MockCoreRuntime);

    let token = kernel
        .issue_token("audit-schema", "agent-schema", 120)
        .expect("token should issue");

    kernel
        .execute_runtime_core(
            "audit-schema",
            &token,
            &BTreeSet::from([Capability::ObserveTelemetry]),
            None,
            RuntimeCoreRequest {
                action: "boot".to_owned(),
                payload: json!({"mode": "safe"}),
            },
        )
        .await
        .expect("runtime core should execute");

    let snapshot = audit.snapshot();
    assert_eq!(snapshot.len(), 2);

    let plane_event_json = serde_json::to_value(snapshot.last().expect("plane event should exist"))
        .expect("serialize event");
    assert_eq!(
        plane_event_json,
        json!({
            "event_id": "evt-0000000000000002",
            "timestamp_epoch_s": 1_700_001_000_u64,
            "agent_id": "agent-schema",
            "kind": {
                "PlaneInvoked": {
                    "pack_id": "audit-schema",
                    "plane": "Runtime",
                    "tier": "Core",
                    "primary_adapter": "native-core",
                    "delegated_core_adapter": null,
                    "operation": "boot",
                    "required_capabilities": ["ObserveTelemetry"]
                }
            }
        })
    );
}

#[tokio::test]
async fn tool_core_call_is_denied_when_policy_engine_rejects_rule_of_two_gate() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_002_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "tool-gate-deny".to_owned(),
            domain: "security".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
    kernel.register_core_tool_adapter(MockCoreTool);
    kernel.register_policy_extension(ToolGatePolicyExtension::new(
        "shell.exec",
        ToolGateMode::Deny,
    ));

    let token = kernel
        .issue_token("tool-gate-deny", "agent-deny", 120)
        .expect("token should issue");
    let required = BTreeSet::from([Capability::InvokeTool]);

    let error = kernel
        .execute_tool_core(
            "tool-gate-deny",
            &token,
            &required,
            None,
            ToolCoreRequest {
                tool_name: "shell.exec".to_owned(),
                payload: json!({"command": "ls"}),
            },
        )
        .await
        .expect_err("tool call should be denied by policy");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::ToolCallDenied { tool_name, .. })
            if tool_name == "shell.exec"
    ));

    let events = audit.snapshot();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[1].kind,
        AuditEventKind::AuthorizationDenied { pack_id, token_id, reason }
            if pack_id == "tool-gate-deny"
                && token_id == &token.token_id
                && reason.contains("tool call denied by policy")
    ));
}

#[tokio::test]
async fn generation_revoke_below_threshold_denies_old_tokens() {
    let clock = Arc::new(FixedClock::new(1_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);
    kernel.register_pack(sample_pack()).unwrap();
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });

    let token_gen1 = kernel.issue_token("sales-intel", "agent-1", 3600).unwrap();
    assert_eq!(token_gen1.generation, 1);

    // Revoke all tokens at or below generation 1
    kernel.revoke_generation(1);

    let token_gen2 = kernel.issue_token("sales-intel", "agent-2", 3600).unwrap();
    assert_eq!(token_gen2.generation, 2);

    // Old token should fail
    let caps = BTreeSet::from([Capability::InvokeTool]);
    let task = TaskIntent {
        task_id: "t-1".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: caps.clone(),
        payload: json!({}),
    };
    let err = kernel
        .execute_task("sales-intel", &token_gen1, task)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        KernelError::Policy(PolicyError::RevokedToken { .. })
    ));

    // New token should work
    let task2 = TaskIntent {
        task_id: "t-2".to_owned(),
        objective: "test2".to_owned(),
        required_capabilities: caps,
        payload: json!({}),
    };
    let result = kernel.execute_task("sales-intel", &token_gen2, task2).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn task_supervisor_tracks_state_through_lifecycle() {
    let clock = Arc::new(FixedClock::new(1_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);
    kernel.register_pack(sample_pack()).unwrap();
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    let token = kernel.issue_token("sales-intel", "agent-1", 3600).unwrap();

    let intent = TaskIntent {
        task_id: "supervised-1".to_owned(),
        objective: "supervised test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"key": "value"}),
    };

    let mut supervisor = TaskSupervisor::new(intent);
    assert!(supervisor.is_runnable());

    let result = supervisor.execute(&kernel, "sales-intel", &token).await;
    assert!(result.is_ok());
    assert!(matches!(supervisor.state(), TaskState::Completed(_)));
}

#[tokio::test]
async fn task_supervisor_faults_on_kernel_error() {
    let clock = Arc::new(FixedClock::new(1_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);
    kernel.register_pack(sample_pack()).unwrap();
    // NOTE: no harness adapter registered -- execute_task will fail
    let token = kernel.issue_token("sales-intel", "agent-1", 3600).unwrap();

    let intent = TaskIntent {
        task_id: "supervised-fail".to_owned(),
        objective: "will fail".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };

    let mut supervisor = TaskSupervisor::new(intent);
    let result = supervisor.execute(&kernel, "sales-intel", &token).await;
    assert!(result.is_err());
    assert!(matches!(supervisor.state(), TaskState::Faulted(_)));
}

#[test]
fn register_pack_creates_namespace() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel.register_pack(sample_pack()).unwrap();

    let ns = kernel.get_namespace("sales-intel");
    assert!(ns.is_some());
    let ns = ns.unwrap();
    assert_eq!(ns.pack_id, "sales-intel");
    assert_eq!(ns.domain, "sales");
    assert!(ns.granted_capabilities.contains(&Capability::InvokeTool));
}

#[test]
fn get_namespace_returns_none_for_unregistered_pack() {
    let (kernel, _audit) = LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    assert!(kernel.get_namespace("nonexistent").is_none());
}

#[test]
fn namespace_membrane_defaults_to_pack_id() {
    let (mut kernel, _audit) =
        LoongClawKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel.register_pack(sample_pack()).unwrap();

    let ns = kernel.get_namespace("sales-intel").unwrap();
    assert_eq!(ns.membrane, "sales-intel");
}

#[tokio::test]
async fn kernel_is_usable_from_concurrent_tasks() {
    let (mut builder, _audit) =
        KernelBuilder::new_with_in_memory_audit(StaticPolicyEngine::default());
    builder
        .register_pack(sample_pack())
        .expect("pack should register");
    builder.register_core_connector_adapter(MockCoreConnector);
    builder
        .set_default_core_connector_adapter("http-core")
        .expect("default connector adapter should register");
    let kernel = Arc::new(builder.build());

    let token = kernel
        .issue_token("sales-intel", "agent-alpha", 120)
        .expect("token should issue");

    let mut handles = Vec::new();
    for i in 0..4 {
        let kernel = Arc::clone(&kernel);
        let token = token.clone();
        handles.push(tokio::spawn(async move {
            kernel
                .execute_connector_core(
                    "sales-intel",
                    &token,
                    None,
                    ConnectorCommand {
                        connector_name: "crm".to_owned(),
                        operation: "lookup".to_owned(),
                        required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        payload: json!({"id": format!("concurrent-{i}")}),
                    },
                )
                .await
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let dispatch = handle
            .await
            .expect("task should not panic")
            .expect("concurrent dispatch should succeed");
        assert_eq!(dispatch.connector_name, "crm");
        assert_eq!(dispatch.outcome.status, "ok");
        assert_eq!(
            dispatch.outcome.payload,
            json!({
                "tier": "core",
                "adapter": "http-core",
                "connector": "crm",
                "operation": "lookup",
                "payload": {"id": format!("concurrent-{i}")},
            })
        );
    }
}
