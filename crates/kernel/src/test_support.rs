#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use crate::connector::{ConnectorExtensionAdapter, CoreConnectorAdapter};
use crate::contracts::{
    Capability, ConnectorCommand, ConnectorOutcome, ExecutionRoute, HarnessKind, HarnessOutcome,
    HarnessRequest,
};
use crate::errors::{ConnectorError, PolicyError};
use crate::harness::HarnessAdapter;
use crate::memory::{
    CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest,
};
use crate::pack::VerticalPackManifest;
use crate::policy_ext::{PolicyExtension, PolicyExtensionContext};
use crate::runtime::{
    CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
    RuntimeExtensionOutcome, RuntimeExtensionRequest,
};
use crate::tool::{
    CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome,
    ToolExtensionRequest,
};

pub struct MockEmbeddedPiHarness {
    pub seen_tasks: Mutex<Vec<String>>,
}
pub struct MockCrmConnector;
pub struct MockCoreConnector;
pub struct MockCoreConnectorGrpc;
pub struct MockConnectorExtension;
pub struct MockAcpHarness;
pub struct MockCoreRuntime;
pub struct MockCoreRuntimeFallback;
pub struct MockRuntimeExtension;
pub struct MockCoreTool;
pub struct MockToolExtension;
pub struct MockCoreMemory;
pub struct MockMemoryExtension;
pub struct NoNetworkEgressPolicyExtension;
pub const TEST_CAPABILITY_VARIANTS: [Capability; 8] = [
    Capability::InvokeTool,
    Capability::InvokeConnector,
    Capability::MemoryRead,
    Capability::MemoryWrite,
    Capability::FilesystemRead,
    Capability::FilesystemWrite,
    Capability::NetworkEgress,
    Capability::ObserveTelemetry,
];
pub const TEST_CAPABILITY_VARIANT_COUNT: u8 = TEST_CAPABILITY_VARIANTS.len() as u8;
#[derive(Debug, Clone, Copy)]
pub enum ToolGateMode {
    Deny,
}
#[derive(Debug)]
pub struct ToolGatePolicyExtension {
    pub gated_tool: String,
    pub mode: ToolGateMode,
}
impl ToolGatePolicyExtension {
    pub fn new(gated_tool: &str, mode: ToolGateMode) -> Self {
        Self {
            gated_tool: gated_tool.to_owned(),
            mode,
        }
    }
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
            output: json!({"adapter":"pi-local","task_id":request.task_id,"objective":request.objective}),
        })
    }
}
#[async_trait]
impl CoreConnectorAdapter for MockCrmConnector {
    fn name(&self) -> &str {
        "crm"
    }
    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({"operation":command.operation,"echo":command.payload}),
        })
    }
}
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
            payload: json!({"tier":"core","adapter":"http-core","connector":command.connector_name,"operation":command.operation,"payload":command.payload}),
        })
    }
}
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
            payload: json!({"tier":"core","adapter":"grpc-core","connector":command.connector_name,"operation":command.operation}),
        })
    }
}
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
                payload: json!({"mode":"probe"}),
            })
            .await?;
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({"tier":"extension","extension":"shielded-bridge","operation":command.operation,"core_probe":core_probe.payload,"payload":command.payload}),
        })
    }
}
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
            output: json!({"adapter":"acp-gateway","task_id":request.task_id}),
        })
    }
}
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
            payload: json!({"adapter":"native-core","action":request.action,"payload":request.payload}),
        })
    }
}
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
            payload: json!({"adapter":"fallback-core","action":request.action}),
        })
    }
}
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
            payload: json!({"extension":"acp-bridge","action":request.action,"core_probe":core_probe.payload,"payload":request.payload}),
        })
    }
}
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
            payload: json!({"tool":request.tool_name,"payload":request.payload}),
        })
    }
}
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
            payload: json!({"extension":"sql-analytics","action":request.extension_action,"core_probe":core_probe.payload}),
        })
    }
}
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
            payload: json!({"operation":request.operation,"payload":request.payload}),
        })
    }
}
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
                payload: json!({"key":"seed"}),
            })
            .await?;
        Ok(MemoryExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({"extension":"vector-index","operation":request.operation,"core_probe":core_probe.payload}),
        })
    }
}
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
impl PolicyExtension for ToolGatePolicyExtension {
    fn name(&self) -> &str {
        "tool-gate"
    }
    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        let Some(params) = context.request_parameters else {
            return Ok(());
        };
        let tool_name = params
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if tool_name != self.gated_tool {
            return Ok(());
        }
        match self.mode {
            ToolGateMode::Deny => Err(PolicyError::ToolCallDenied {
                tool_name: tool_name.to_owned(),
                reason: "blocked by deterministic policy rule".to_owned(),
            }),
        }
    }
}
pub fn sample_pack() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: "sales-intel".to_owned(),
        domain: "sales".to_owned(),
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
        ]),
        metadata: BTreeMap::from([("owner".to_owned(), "revenue-team".to_owned())]),
    }
}
pub fn acp_pack_without_explicit_adapter() -> VerticalPackManifest {
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
pub fn capability_from_bit(bit: u8) -> Capability {
    let bit_index = usize::from(bit);
    TEST_CAPABILITY_VARIANTS
        .get(bit_index)
        .copied()
        .expect("test capability bit should be in range")
}
pub fn capability_set_from_mask(mask: u16) -> BTreeSet<Capability> {
    let mut capabilities = BTreeSet::new();
    for (bit_index, capability) in TEST_CAPABILITY_VARIANTS.iter().copied().enumerate() {
        let bit_mask = 1_u16 << bit_index;
        let is_enabled = (mask & bit_mask) != 0;
        if is_enabled {
            capabilities.insert(capability);
        }
    }
    capabilities
}
