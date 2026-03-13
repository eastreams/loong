use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use proptest::prelude::*;
use serde_json::json;

use crate::task_supervisor::TaskSupervisor;
use crate::{
    Fault, TaskState,
    audit::{AuditEventKind, ExecutionPlane, InMemoryAuditSink, PlaneTier},
    clock::FixedClock,
    connector::{ConnectorAdapter, ConnectorExtensionAdapter, CoreConnectorAdapter},
    contracts::{
        Capability, ConnectorCommand, ConnectorOutcome, ExecutionRoute, HarnessKind,
        HarnessOutcome, HarnessRequest, TaskIntent,
    },
    errors::{ConnectorError, KernelError, PolicyError},
    harness::HarnessAdapter,
    kernel::LoongClawKernel,
    memory::{
        CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
        MemoryExtensionOutcome, MemoryExtensionRequest,
    },
    pack::VerticalPackManifest,
    policy::{PolicyDecision, PolicyEngine, PolicyRequest, StaticPolicyEngine},
    policy_ext::{PolicyExtension, PolicyExtensionContext},
    runtime::{
        CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
        RuntimeExtensionOutcome, RuntimeExtensionRequest,
    },
    tool::{
        CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter,
        ToolExtensionOutcome, ToolExtensionRequest,
    },
};

struct MockEmbeddedPiHarness {
    seen_tasks: Mutex<Vec<String>>,
}

#[async_trait]
impl HarnessAdapter for MockEmbeddedPiHarness {
    fn name(&self) -> &str {
        "pi-local"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::EmbeddedPi
    }

    async fn execute(
        &self,
        request: HarnessRequest,
    ) -> Result<HarnessOutcome, crate::HarnessError> {
        self.seen_tasks
            .lock()
            .expect("mutex poisoned")
            .push(request.task_id.clone());

        Ok(HarnessOutcome {
            status: "ok".to_owned(),
            output: json!({
                "adapter": "pi-local",
                "task_id": request.task_id,
                "objective": request.objective,
            }),
        })
    }
}

struct MockCrmConnector;

#[async_trait]
impl ConnectorAdapter for MockCrmConnector {
    fn name(&self) -> &str {
        "crm"
    }

    async fn invoke(&self, command: ConnectorCommand) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "operation": command.operation,
                "echo": command.payload,
            }),
        })
    }
}

struct MockCoreConnector;

#[async_trait]
impl CoreConnectorAdapter for MockCoreConnector {
    fn name(&self) -> &str {
        "http-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "http-core",
                "connector": command.connector_name,
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

struct MockCoreConnectorGrpc;

#[async_trait]
impl CoreConnectorAdapter for MockCoreConnectorGrpc {
    fn name(&self) -> &str {
        "grpc-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "grpc-core",
                "connector": command.connector_name,
                "operation": command.operation,
            }),
        })
    }
}

struct MockConnectorExtension;

#[async_trait]
impl ConnectorExtensionAdapter for MockConnectorExtension {
    fn name(&self) -> &str {
        "shielded-bridge"
    }

    async fn invoke_extension(
        &self,
        command: ConnectorCommand,
        core: &(dyn CoreConnectorAdapter + Sync),
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let core_probe = core
            .invoke_core(ConnectorCommand {
                connector_name: command.connector_name.clone(),
                operation: "probe".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({"mode": "probe"}),
            })
            .await?;

        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "extension",
                "extension": "shielded-bridge",
                "operation": command.operation,
                "core_probe": core_probe.payload,
                "payload": command.payload,
            }),
        })
    }
}

struct MockAcpHarness;

#[async_trait]
impl HarnessAdapter for MockAcpHarness {
    fn name(&self) -> &str {
        "acp-gateway"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::Acp
    }

    async fn execute(
        &self,
        request: HarnessRequest,
    ) -> Result<HarnessOutcome, crate::HarnessError> {
        Ok(HarnessOutcome {
            status: "ok".to_owned(),
            output: json!({
                "adapter": "acp-gateway",
                "task_id": request.task_id,
            }),
        })
    }
}

fn sample_pack() -> VerticalPackManifest {
    let allowed_connectors = BTreeSet::from(["crm".to_owned()]);
    let granted_capabilities = BTreeSet::from([
        Capability::InvokeTool,
        Capability::InvokeConnector,
        Capability::MemoryRead,
    ]);

    VerticalPackManifest {
        pack_id: "sales-intel".to_owned(),
        domain: "sales".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors,
        granted_capabilities,
        metadata: BTreeMap::from([("owner".to_owned(), "revenue-team".to_owned())]),
    }
}

fn acp_pack_without_explicit_adapter() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: "code-review".to_owned(),
        domain: "engineering".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::Acp,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
        metadata: BTreeMap::new(),
    }
}

fn capability_from_bit(bit: u8) -> Capability {
    match bit {
        0 => Capability::InvokeTool,
        1 => Capability::InvokeConnector,
        2 => Capability::MemoryRead,
        3 => Capability::MemoryWrite,
        4 => Capability::FilesystemRead,
        5 => Capability::FilesystemWrite,
        6 => Capability::NetworkEgress,
        7 => Capability::ScheduleTask,
        _ => Capability::ObserveTelemetry,
    }
}

fn capability_set_from_mask(mask: u16) -> BTreeSet<Capability> {
    let mut capabilities = BTreeSet::new();
    for bit in 0_u8..9 {
        if (mask & (1_u16 << bit)) != 0 {
            capabilities.insert(capability_from_bit(bit));
        }
    }
    capabilities
}

#[tokio::test]
async fn kernel_executes_task_and_connector_under_pack_policy() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    kernel.register_connector(MockCrmConnector);

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
        .invoke_connector("sales-intel", &token, connector_command)
        .await
        .expect("connector dispatch should succeed");

    assert_eq!(connector_dispatch.connector_name, "crm");
    assert_eq!(connector_dispatch.outcome.status, "ok");
}

#[tokio::test]
async fn kernel_rejects_token_missing_capability() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");
    kernel.register_connector(MockCrmConnector);

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
        .invoke_connector("sales-intel", &token, command)
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel.register_core_connector_adapter(MockCoreConnector);

    let error = kernel
        .set_default_core_connector_adapter("missing-core")
        .expect_err("missing adapter should fail");

    assert!(matches!(
        error,
        KernelError::Connector(ConnectorError::CoreAdapterNotFound(name)) if name == "missing-core"
    ));
}

#[test]
fn pack_validation_rejects_invalid_semver() {
    let mut pack = sample_pack();
    pack.version = "version-one".to_owned();

    let error = pack.validate().expect_err("invalid semver should fail");
    assert!(matches!(error, crate::PackError::InvalidVersion(_)));
}

#[tokio::test]
async fn kernel_auto_routes_by_harness_kind_when_adapter_is_not_pinned() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    kernel.register_connector(MockCrmConnector);

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
        .invoke_connector(
            "sales-intel",
            &token,
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
                tier: PlaneTier::Legacy,
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
    let kernel =
        LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock.clone(), audit.clone());

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

struct MockCoreRuntime;

#[async_trait]
impl CoreRuntimeAdapter for MockCoreRuntime {
    fn name(&self) -> &str {
        "native-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, crate::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "native-core",
                "action": request.action,
                "payload": request.payload,
            }),
        })
    }
}

struct MockCoreRuntimeFallback;

#[async_trait]
impl CoreRuntimeAdapter for MockCoreRuntimeFallback {
    fn name(&self) -> &str {
        "fallback-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, crate::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "fallback-core",
                "action": request.action,
            }),
        })
    }
}

struct MockRuntimeExtension;

#[async_trait]
impl RuntimeExtensionAdapter for MockRuntimeExtension {
    fn name(&self) -> &str {
        "acp-bridge"
    }

    async fn execute_extension(
        &self,
        request: RuntimeExtensionRequest,
        core: &(dyn CoreRuntimeAdapter + Sync),
    ) -> Result<RuntimeExtensionOutcome, crate::RuntimePlaneError> {
        let core_probe = core
            .execute_core(RuntimeCoreRequest {
                action: "probe".to_owned(),
                payload: json!({}),
            })
            .await?;

        Ok(RuntimeExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "acp-bridge",
                "action": request.action,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

struct MockCoreTool;

#[async_trait]
impl CoreToolAdapter for MockCoreTool {
    fn name(&self) -> &str {
        "core-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, crate::ToolPlaneError> {
        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tool": request.tool_name,
                "payload": request.payload,
            }),
        })
    }
}

struct MockToolExtension;

#[async_trait]
impl ToolExtensionAdapter for MockToolExtension {
    fn name(&self) -> &str {
        "sql-analytics"
    }

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, crate::ToolPlaneError> {
        let core_probe = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "schema_probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "sql-analytics",
                "action": request.extension_action,
                "core_probe": core_probe.payload,
            }),
        })
    }
}

struct MockCoreMemory;

#[async_trait]
impl CoreMemoryAdapter for MockCoreMemory {
    fn name(&self) -> &str {
        "kv-core"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, crate::MemoryPlaneError> {
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "operation": request.operation,
                "payload": request.payload,
            }),
        })
    }
}

struct MockMemoryExtension;

#[async_trait]
impl MemoryExtensionAdapter for MockMemoryExtension {
    fn name(&self) -> &str {
        "vector-index"
    }

    async fn execute_memory_extension(
        &self,
        request: MemoryExtensionRequest,
        core: &(dyn CoreMemoryAdapter + Sync),
    ) -> Result<MemoryExtensionOutcome, crate::MemoryPlaneError> {
        let core_probe = core
            .execute_core_memory(MemoryCoreRequest {
                operation: "read".to_owned(),
                payload: json!({ "key": "seed" }),
            })
            .await?;
        Ok(MemoryExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "vector-index",
                "operation": request.operation,
                "core_probe": core_probe.payload,
            }),
        })
    }
}

struct NoNetworkEgressPolicyExtension;

impl PolicyExtension for NoNetworkEgressPolicyExtension {
    fn name(&self) -> &str {
        "no-network-egress"
    }

    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        if context
            .required_capabilities
            .contains(&Capability::NetworkEgress)
        {
            return Err(PolicyError::ExtensionDenied {
                extension: self.name().to_owned(),
                reason: "network egress is blocked for this environment".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolGateMode {
    Deny,
    RequireApproval,
}

#[derive(Debug)]
struct ToolGatePolicyEngine {
    base: StaticPolicyEngine,
    gated_tool: String,
    mode: ToolGateMode,
}

impl ToolGatePolicyEngine {
    fn new(gated_tool: &str, mode: ToolGateMode) -> Self {
        Self {
            base: StaticPolicyEngine::default(),
            gated_tool: gated_tool.to_owned(),
            mode,
        }
    }
}

impl PolicyEngine for ToolGatePolicyEngine {
    fn issue_token(
        &self,
        pack: &VerticalPackManifest,
        agent_id: &str,
        now_epoch_s: u64,
        ttl_s: u64,
    ) -> Result<crate::CapabilityToken, PolicyError> {
        self.base.issue_token(pack, agent_id, now_epoch_s, ttl_s)
    }

    fn authorize(
        &self,
        token: &crate::CapabilityToken,
        runtime_pack_id: &str,
        now_epoch_s: u64,
        required: &BTreeSet<Capability>,
    ) -> Result<(), PolicyError> {
        self.base
            .authorize(token, runtime_pack_id, now_epoch_s, required)
    }

    fn revoke_token(&self, token_id: &str) -> Result<(), PolicyError> {
        self.base.revoke_token(token_id)
    }

    fn check_tool_call(&self, request: &PolicyRequest) -> PolicyDecision {
        if request.tool_name != self.gated_tool {
            return PolicyDecision::Allow;
        }

        match self.mode {
            ToolGateMode::Deny => {
                PolicyDecision::Deny("blocked by deterministic policy rule".to_owned())
            }
            ToolGateMode::RequireApproval => {
                PolicyDecision::RequireApproval("manual approval required for this tool".to_owned())
            }
        }
    }
}

#[tokio::test]
async fn layered_runtime_tool_and_memory_paths_execute_via_core_and_extension() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let mut kernel = LoongClawKernel::with_runtime(
        ToolGatePolicyEngine::new("shell.exec", ToolGateMode::Deny),
        clock.clone(),
        audit.clone(),
    );
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
async fn tool_extension_call_reports_approval_required_when_policy_requires_human_gate() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_003_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(
        ToolGatePolicyEngine::new("query_ledger", ToolGateMode::RequireApproval),
        clock.clone(),
        audit.clone(),
    );
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: "tool-gate-approval".to_owned(),
            domain: "finance".to_owned(),
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
    kernel.register_tool_extension_adapter(MockToolExtension);

    let token = kernel
        .issue_token("tool-gate-approval", "agent-approval", 120)
        .expect("token should issue");
    let required = BTreeSet::from([Capability::InvokeTool]);

    let error = kernel
        .execute_tool_extension(
            "tool-gate-approval",
            &token,
            &required,
            "sql-analytics",
            None,
            ToolExtensionRequest {
                extension_action: "query_ledger".to_owned(),
                payload: json!({"sql": "select * from ledger"}),
            },
        )
        .await
        .expect_err("tool extension should require approval before execution");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::ToolCallApprovalRequired { tool_name, .. })
            if tool_name == "query_ledger"
    ));

    let events = audit.snapshot();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[1].kind,
        AuditEventKind::AuthorizationDenied { pack_id, token_id, reason }
            if pack_id == "tool-gate-approval"
                && token_id == &token.token_id
                && reason.contains("requires approval")
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

#[test]
fn token_generation_increments_on_each_issue() {
    let engine = StaticPolicyEngine::default();
    let pack = sample_pack();
    let t1 = engine.issue_token(&pack, "a1", 1_000_000, 3600).unwrap();
    let t2 = engine.issue_token(&pack, "a2", 1_000_000, 3600).unwrap();
    let t3 = engine.issue_token(&pack, "a3", 1_000_000, 3600).unwrap();
    assert_eq!(t1.generation, 1);
    assert_eq!(t2.generation, 2);
    assert_eq!(t3.generation, 3);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_pack_capability_boundary_for_task_dispatch(
        pack_mask in 1_u16..(1_u16 << 9),
        required_mask in 0_u16..(1_u16 << 9)
    ) {
        let pack_capabilities = capability_set_from_mask(pack_mask);
        let required_capabilities = capability_set_from_mask(required_mask);

        let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
        let mut pack = sample_pack();
        pack.granted_capabilities = pack_capabilities.clone();
        kernel
            .register_pack(pack)
            .expect("pack should register");
        kernel.register_harness_adapter(MockEmbeddedPiHarness {
            seen_tasks: Mutex::new(Vec::new()),
        });

        let token = kernel
            .issue_token("sales-intel", "agent-prop", 120)
            .expect("token should issue");

        let task = TaskIntent {
            task_id: "task-prop".to_owned(),
            objective: "property boundary check".to_owned(),
            required_capabilities: required_capabilities.clone(),
            payload: json!({}),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime should build");
        let result = runtime.block_on(kernel.execute_task("sales-intel", &token, task));

        if required_capabilities.is_subset(&pack_capabilities) {
            prop_assert!(result.is_ok());
        } else {
            let boundary_error = matches!(result, Err(KernelError::PackCapabilityBoundary { .. }));
            prop_assert!(boundary_error);
        }
    }
}

// ---------------------------------------------------------------------------
// Fault enum tests
// ---------------------------------------------------------------------------

#[test]
fn fault_display_is_human_readable() {
    let fault = Fault::CapabilityViolation {
        token_id: "tok-1".to_owned(),
        capability: Capability::InvokeTool,
    };
    let msg = fault.to_string();
    assert!(msg.contains("tok-1"));
    assert!(msg.contains("InvokeTool"));
}

#[test]
fn fault_from_policy_error_maps_expired_token() {
    let policy_err = PolicyError::ExpiredToken {
        token_id: "tok-2".to_owned(),
        expires_at_epoch_s: 1000,
    };
    let fault = Fault::from_policy_error(policy_err);
    assert!(
        matches!(fault, Fault::TokenExpired { token_id, expires_at_epoch_s } if token_id == "tok-2" && expires_at_epoch_s == 1000)
    );
}

#[test]
fn fault_from_policy_error_maps_missing_capability() {
    let policy_err = PolicyError::MissingCapability {
        token_id: "tok-3".to_owned(),
        capability: Capability::MemoryWrite,
    };
    let fault = Fault::from_policy_error(policy_err);
    assert!(matches!(fault, Fault::CapabilityViolation { .. }));
}

#[test]
fn fault_from_kernel_error_maps_policy() {
    let kernel_err = KernelError::Policy(PolicyError::RevokedToken {
        token_id: "tok-4".to_owned(),
    });
    let fault = Fault::from_kernel_error(kernel_err);
    assert!(matches!(fault, Fault::PolicyDenied { .. }));
}

#[test]
fn fault_from_kernel_error_maps_pack_boundary() {
    let kernel_err = KernelError::PackCapabilityBoundary {
        pack_id: "my-pack".to_owned(),
        capability: Capability::NetworkEgress,
    };
    let fault = Fault::from_kernel_error(kernel_err);
    assert!(matches!(fault, Fault::CapabilityViolation { .. }));
}

#[test]
fn fault_panic_carries_message() {
    let fault = Fault::Panic {
        message: "unexpected state".to_owned(),
    };
    assert!(fault.to_string().contains("unexpected state"));
}

// ── TaskState FSM tests ──────────────────────────────────────────────

#[test]
fn task_state_transitions_runnable_to_in_send() {
    let intent = TaskIntent {
        task_id: "t-1".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let state = TaskState::Runnable(intent);
    let next = state.transition_to_in_send();
    assert!(next.is_ok());
    assert!(matches!(next.unwrap(), TaskState::InSend { .. }));
}

#[test]
fn task_state_rejects_invalid_transition_from_completed() {
    let state = TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    });
    let err = state.transition_to_in_send();
    assert!(err.is_err());
}

#[test]
fn task_state_faulted_carries_fault() {
    let fault = Fault::Panic {
        message: "boom".to_owned(),
    };
    let state = TaskState::Faulted(fault.clone());
    if let TaskState::Faulted(f) = state {
        assert_eq!(f, fault);
    } else {
        panic!("expected Faulted");
    }
}

#[test]
fn task_state_full_transition_chain() {
    let intent = TaskIntent {
        task_id: "t-chain".to_owned(),
        objective: "chain test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let state = TaskState::Runnable(intent);
    let state = state.transition_to_in_send().unwrap();
    assert!(matches!(state, TaskState::InSend { .. }));
    let state = state.transition_to_in_reply().unwrap();
    assert!(matches!(state, TaskState::InReply { .. }));
    let outcome = HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({"result": "done"}),
    };
    let state = state.transition_to_completed(outcome).unwrap();
    assert!(matches!(state, TaskState::Completed(_)));
    assert!(state.is_terminal());
}

#[test]
fn task_state_faulted_from_non_terminal_succeeds() {
    let state = TaskState::InSend {
        task_id: "t-fault".to_owned(),
    };
    let fault = Fault::Panic {
        message: "oops".to_owned(),
    };
    let state = state.transition_to_faulted(fault);
    assert!(matches!(state, TaskState::Faulted(_)));
}

#[test]
fn task_state_faulted_from_terminal_is_noop() {
    let state = TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    });
    let fault = Fault::Panic {
        message: "late".to_owned(),
    };
    let state = state.transition_to_faulted(fault);
    // Should remain Completed, not change to Faulted
    assert!(matches!(state, TaskState::Completed(_)));
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
fn task_supervisor_rejects_execute_after_completion() {
    let intent = TaskIntent {
        task_id: "t-double".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let mut supervisor = TaskSupervisor::new(intent);
    supervisor.force_state(TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    }));
    assert!(!supervisor.is_runnable());
}

// ── Namespace tests ──────────────────────────────────────────────────

#[test]
fn register_pack_creates_namespace() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
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
    let kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    assert!(kernel.get_namespace("nonexistent").is_none());
}

#[test]
fn namespace_membrane_defaults_to_pack_id() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel.register_pack(sample_pack()).unwrap();

    let ns = kernel.get_namespace("sales-intel").unwrap();
    assert_eq!(ns.membrane, "sales-intel");
}
