use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, HarnessKind, InMemoryAuditSink,
    Kernel as FrozenKernel, KernelBuilder as RuntimeKernelBuilder, LoongClawKernel,
    StaticPolicyEngine, SystemClock, VerticalPackManifest,
};

use crate::DEFAULT_PACK_ID;
use crate::spec_runtime::{
    AcpBridgeRuntimeExtension, ClawMigrationToolExtension, CoreToolRuntime, CrmCoreConnector,
    CrmGrpcCoreConnector, EmbeddedPiHarness, FallbackCoreRuntime, KvCoreMemory, NativeCoreRuntime,
    ShieldedConnectorExtension, SqlAnalyticsToolExtension, VectorIndexMemoryExtension,
    WebhookConnector,
};

/// The spec/bootstrap layer is a harness-facing surface, so its default audit
/// sink stays explicitly in-memory unless a caller wires a different sink.
pub fn default_in_memory_audit_sink() -> Arc<InMemoryAuditSink> {
    Arc::new(InMemoryAuditSink::default())
}

/// Builder for constructing a fully configured `LoongClawKernel`.
///
/// By default the builder uses `SystemClock` and the spec layer's named
/// in-memory audit helper. Override either with the corresponding setter before
/// calling `build()`.
#[derive(Default)]
pub struct KernelBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
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

    pub fn native_tool_executor(mut self, executor: crate::NativeToolExecutor) -> Self {
        self.native_tool_executor = Some(executor);
        self
    }

    /// Build and return a fully configured kernel with all builtin adapters
    /// and the default pack manifest registered.
    pub fn build(self) -> LoongClawKernel<StaticPolicyEngine> {
        configured_builder(self.clock, self.audit, self.native_tool_executor)
    }
}

/// Additive bootstrap entrypoint that exposes the new builder/runtime split
/// without breaking the legacy `KernelBuilder` API.
///
/// The returned runtime handle dereferences to the legacy kernel surface so
/// helper code typed against `&LoongClawKernel<_>` can continue to work while
/// callers migrate toward the explicit `Kernel<P>` name.
#[derive(Default)]
pub struct BootstrapBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
}

impl BootstrapBuilder {
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn native_tool_executor(mut self, executor: crate::NativeToolExecutor) -> Self {
        self.native_tool_executor = Some(executor);
        self
    }

    pub fn build(self) -> FrozenKernel<StaticPolicyEngine> {
        self.into_builder().build()
    }

    /// Return the additive migration builder surface.
    ///
    /// This remains a compatibility alias over `LoongClawKernel`, so it keeps
    /// the legacy executable API while also supporting `.build()` into
    /// `Kernel<P>`.
    pub fn into_builder(self) -> RuntimeKernelBuilder<StaticPolicyEngine> {
        configured_builder(self.clock, self.audit, self.native_tool_executor)
    }
}

fn configured_builder(
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
) -> RuntimeKernelBuilder<StaticPolicyEngine> {
    let mut kernel = match (clock, audit) {
        (Some(clock), Some(audit)) => {
            RuntimeKernelBuilder::with_runtime(StaticPolicyEngine::default(), clock, audit)
        }
        (Some(clock), None) => RuntimeKernelBuilder::with_runtime(
            StaticPolicyEngine::default(),
            clock,
            default_in_memory_audit_sink() as Arc<dyn AuditSink>,
        ),
        (None, Some(audit)) => RuntimeKernelBuilder::with_runtime(
            StaticPolicyEngine::default(),
            Arc::new(SystemClock) as Arc<dyn Clock>,
            audit,
        ),
        (None, None) => RuntimeKernelBuilder::with_runtime(
            StaticPolicyEngine::default(),
            Arc::new(SystemClock) as Arc<dyn Clock>,
            default_in_memory_audit_sink() as Arc<dyn AuditSink>,
        ),
    };
    register_builtin_adapters(&mut kernel, native_tool_executor);
    // The default pack manifest is hardcoded and always valid; ignore the
    // impossible error branch to avoid panicking in production.
    let _ = kernel.register_pack(default_pack_manifest());
    kernel
}

fn register_builtin_adapters(
    kernel: &mut RuntimeKernelBuilder<StaticPolicyEngine>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
) {
    kernel.register_harness_adapter(EmbeddedPiHarness {
        seen: Mutex::new(Vec::new()),
    });
    kernel.register_core_connector_adapter(WebhookConnector);
    kernel.register_core_connector_adapter(CrmCoreConnector);
    kernel.register_core_connector_adapter(CrmGrpcCoreConnector);
    kernel.register_connector_extension_adapter(ShieldedConnectorExtension);

    kernel.register_core_runtime_adapter(NativeCoreRuntime);
    kernel.register_core_runtime_adapter(FallbackCoreRuntime);
    kernel.register_runtime_extension_adapter(AcpBridgeRuntimeExtension);

    kernel.register_core_tool_adapter(CoreToolRuntime::new(native_tool_executor));
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
    use kernel::{AuditEventKind, FixedClock, LoongClawKernel};

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

    #[test]
    fn builder_explicit_in_memory_audit_records_events() {
        let audit = default_in_memory_audit_sink();
        let kernel = KernelBuilder::default()
            .audit(audit.clone() as Arc<dyn AuditSink>)
            .build();

        kernel
            .issue_token(DEFAULT_PACK_ID, "spec-audit-builder", 60)
            .expect("token issue should succeed with the named in-memory audit helper");

        let events = audit.snapshot();
        assert_eq!(events.len(), 1, "expected one token-issued audit event");
        assert!(matches!(events[0].kind, AuditEventKind::TokenIssued { .. }));
    }

    #[test]
    fn into_builder_allows_extra_registration_before_freeze() {
        let mut builder = BootstrapBuilder::default().into_builder();
        builder
            .register_pack(VerticalPackManifest {
                pack_id: "extra-pack".to_owned(),
                domain: "engineering".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            })
            .expect("extra pack should register");

        let kernel = builder.build();
        let token = kernel
            .issue_token("extra-pack", "test-agent", 60)
            .expect("token issue should succeed for extra pack");
        assert!(!token.token_id.is_empty());
    }

    #[test]
    fn bootstrap_builder_runtime_derefs_to_legacy_kernel_helpers() {
        fn issue_default_pack_token(
            kernel: &LoongClawKernel<StaticPolicyEngine>,
        ) -> kernel::CapabilityToken {
            kernel
                .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
                .expect("token issue should succeed via legacy helper signature")
        }

        let kernel = BootstrapBuilder::default().build();
        let token = issue_default_pack_token(&kernel);
        assert!(!token.token_id.is_empty());
    }
}
