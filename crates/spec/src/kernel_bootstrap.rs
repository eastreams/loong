use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, HarnessKind, InMemoryAuditSink, LoongClawKernel,
    StaticPolicyEngine, SystemClock, VerticalPackManifest,
};

use crate::DEFAULT_PACK_ID;
use crate::spec_runtime::{
    AcpBridgeRuntimeExtension, ClawMigrationToolExtension, CoreToolRuntime, CrmCoreConnector,
    CrmGrpcCoreConnector, EmbeddedPiHarness, FallbackCoreRuntime, KvCoreMemory, NativeCoreRuntime,
    ShieldedConnectorExtension, SqlAnalyticsToolExtension, VectorIndexMemoryExtension,
    WebhookConnector,
};

/// Builder for constructing a fully configured `LoongClawKernel`.
///
/// By default the builder uses `SystemClock` and `InMemoryAuditSink`.
/// Override either with the corresponding setter before calling `build()`.
#[derive(Default)]
pub struct KernelBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
}

impl KernelBuilder {
    /// Set a custom `Clock` implementation.
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Set a custom `AuditSink` implementation.
    pub fn audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Build and return a fully configured kernel with all builtin adapters
    /// and the default pack manifest registered.
    pub fn build(self) -> LoongClawKernel<StaticPolicyEngine> {
        let mut kernel = match (self.clock, self.audit) {
            (Some(clock), Some(audit)) => {
                LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit)
            }
            (Some(clock), None) => LoongClawKernel::with_runtime(
                StaticPolicyEngine::default(),
                clock,
                Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
            ),
            (None, Some(audit)) => LoongClawKernel::with_runtime(
                StaticPolicyEngine::default(),
                Arc::new(SystemClock) as Arc<dyn Clock>,
                audit,
            ),
            (None, None) => LoongClawKernel::with_runtime(
                StaticPolicyEngine::default(),
                Arc::new(SystemClock) as Arc<dyn Clock>,
                Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
            ),
        };
        register_builtin_adapters(&mut kernel);
        // The default pack manifest is hardcoded and always valid; ignore the
        // impossible error branch to avoid panicking in production.
        let _ = kernel.register_pack(default_pack_manifest());
        kernel
    }
}

fn register_builtin_adapters(kernel: &mut LoongClawKernel<StaticPolicyEngine>) {
    kernel.register_harness_adapter(EmbeddedPiHarness {
        seen: Mutex::new(Vec::new()),
    });
    kernel.register_connector(WebhookConnector);
    kernel.register_core_connector_adapter(CrmCoreConnector);
    kernel.register_core_connector_adapter(CrmGrpcCoreConnector);
    kernel.register_connector_extension_adapter(ShieldedConnectorExtension);

    kernel.register_core_runtime_adapter(NativeCoreRuntime);
    kernel.register_core_runtime_adapter(FallbackCoreRuntime);
    kernel.register_runtime_extension_adapter(AcpBridgeRuntimeExtension);

    kernel.register_core_tool_adapter(CoreToolRuntime);
    kernel.register_tool_extension_adapter(ClawMigrationToolExtension);
    kernel.register_tool_extension_adapter(SqlAnalyticsToolExtension);

    kernel.register_core_memory_adapter(KvCoreMemory);
    kernel.register_memory_extension_adapter(VectorIndexMemoryExtension);
}

/// Construct the default vertical pack manifest used during bootstrap.
///
/// Exposed as a standalone function so other modules (e.g. tests) can reuse it
/// without going through the builder.
pub fn default_pack_manifest() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: DEFAULT_PACK_ID.to_owned(),
        domain: "engineering".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::from([
            "webhook".to_owned(),
            "crm".to_owned(),
            "erp".to_owned(),
        ]),
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::InvokeConnector,
            Capability::MemoryRead,
            Capability::MemoryWrite,
            Capability::ObserveTelemetry,
        ]),
        metadata: BTreeMap::from([
            ("owner".to_owned(), "platform-team".to_owned()),
            ("stage".to_owned(), "prototype".to_owned()),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::FixedClock;

    #[test]
    fn builder_default_creates_kernel() {
        let kernel = KernelBuilder::default().build();
        // Verify the kernel can issue a token for the default pack, which proves
        // the pack was registered and the kernel is functional.
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
            .expect("token issue should succeed on a properly bootstrapped kernel");
        assert!(!token.token_id.is_empty());
    }

    #[test]
    fn builder_with_custom_clock_and_audit() {
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let audit = Arc::new(InMemoryAuditSink::default());
        let kernel = KernelBuilder::default().clock(clock).audit(audit).build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
            .expect("token issue should succeed with custom clock/audit");
        assert!(!token.token_id.is_empty());
    }
}
