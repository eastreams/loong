use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, HarnessKind, InMemoryAuditSink,
    Kernel as FrozenKernel, KernelBuilder as RuntimeKernelBuilder, LoongKernel, StaticPolicyEngine,
    SystemClock, VerticalPackManifest,
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
pub(crate) fn default_in_memory_audit_sink() -> Arc<InMemoryAuditSink> {
    Arc::new(InMemoryAuditSink::default())
}

/// Builder for constructing a fully configured `LoongKernel`.
///
/// By default the builder uses `SystemClock` and the spec layer's named
/// in-memory audit helper. Override either with the corresponding setter before
/// calling `build()`.
pub struct KernelBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    register_default_embedded_harness: bool,
}

impl KernelBuilder {
    pub fn new() -> Self {
        Self {
            clock: None,
            audit: None,
            native_tool_executor: None,
            register_default_embedded_harness: true,
        }
    }

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

    pub fn without_default_embedded_harness(mut self) -> Self {
        self.register_default_embedded_harness = false;
        self
    }

    /// Build and return a fully configured kernel with all builtin adapters
    /// and the default pack manifest registered.
    pub fn build(self) -> LoongKernel<StaticPolicyEngine> {
        configured_builder(
            self.clock,
            self.audit,
            self.native_tool_executor,
            self.register_default_embedded_harness,
        )
    }
}

impl Default for KernelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Additive bootstrap entrypoint that exposes the new builder/runtime split
/// without breaking the legacy `KernelBuilder` API.
///
/// The returned runtime handle dereferences to the legacy kernel surface so
/// helper code typed against `&LoongKernel<_>` can continue to work while
/// callers migrate toward the explicit `Kernel<P>` name.
pub struct BootstrapBuilder {
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    register_default_embedded_harness: bool,
}

impl BootstrapBuilder {
    pub fn new() -> Self {
        Self {
            clock: None,
            audit: None,
            native_tool_executor: None,
            register_default_embedded_harness: true,
        }
    }

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

    pub fn without_default_embedded_harness(mut self) -> Self {
        self.register_default_embedded_harness = false;
        self
    }

    pub fn build(self) -> FrozenKernel<StaticPolicyEngine> {
        self.into_builder().build()
    }

    /// Return the additive migration builder surface.
    ///
    /// This remains a compatibility alias over `LoongKernel`, so it keeps
    /// the legacy executable API while also supporting `.build()` into
    /// `Kernel<P>`.
    pub fn into_builder(self) -> RuntimeKernelBuilder<StaticPolicyEngine> {
        configured_builder(
            self.clock,
            self.audit,
            self.native_tool_executor,
            self.register_default_embedded_harness,
        )
    }
}

impl Default for BootstrapBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn configured_builder(
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    register_default_embedded_harness: bool,
) -> RuntimeKernelBuilder<StaticPolicyEngine> {
    configured_builder_with_default_audit(
        clock,
        audit,
        native_tool_executor,
        register_default_embedded_harness,
    )
    .0
}

fn configured_builder_with_default_audit(
    clock: Option<Arc<dyn Clock>>,
    audit: Option<Arc<dyn AuditSink>>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    register_default_embedded_harness: bool,
) -> (
    RuntimeKernelBuilder<StaticPolicyEngine>,
    Option<Arc<InMemoryAuditSink>>,
) {
    let (mut kernel, fallback_audit) = match (clock, audit) {
        (Some(clock), Some(audit)) => (
            RuntimeKernelBuilder::with_runtime(StaticPolicyEngine::default(), clock, audit),
            None,
        ),
        (Some(clock), None) => {
            let audit = default_in_memory_audit_sink();
            (
                RuntimeKernelBuilder::with_runtime(
                    StaticPolicyEngine::default(),
                    clock,
                    audit.clone() as Arc<dyn AuditSink>,
                ),
                Some(audit),
            )
        }
        (None, Some(audit)) => (
            RuntimeKernelBuilder::with_runtime(
                StaticPolicyEngine::default(),
                Arc::new(SystemClock) as Arc<dyn Clock>,
                audit,
            ),
            None,
        ),
        (None, None) => {
            let audit = default_in_memory_audit_sink();
            (
                RuntimeKernelBuilder::with_runtime(
                    StaticPolicyEngine::default(),
                    Arc::new(SystemClock) as Arc<dyn Clock>,
                    audit.clone() as Arc<dyn AuditSink>,
                ),
                Some(audit),
            )
        }
    };
    register_builtin_adapters(
        &mut kernel,
        native_tool_executor,
        register_default_embedded_harness,
    );
    // The default pack manifest is hardcoded and always valid; ignore the
    // impossible error branch to avoid panicking in production.
    let _ = kernel.register_pack(default_pack_manifest());
    (kernel, fallback_audit)
}

fn register_builtin_adapters(
    kernel: &mut RuntimeKernelBuilder<StaticPolicyEngine>,
    native_tool_executor: Option<crate::NativeToolExecutor>,
    register_default_embedded_harness: bool,
) {
    if register_default_embedded_harness {
        kernel.register_harness_adapter(EmbeddedPiHarness {
            seen: Mutex::new(Vec::new()),
        });
    }
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
    use kernel::{AuditEventKind, FixedClock, LoongKernel};

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
    fn builder_default_fallback_records_token_audit_events() {
        let (kernel, audit) = configured_builder_with_default_audit(None, None, None, true);
        let audit =
            audit.expect("default builder should surface the fallback in-memory audit sink");

        kernel
            .issue_token(DEFAULT_PACK_ID, "spec-audit-builder", 60)
            .expect("token issue should succeed with the default in-memory audit fallback");

        let events = audit.snapshot();
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AuditEventKind::TokenIssued { .. })),
            "expected token issuance to be recorded by the fallback in-memory audit sink"
        );
    }

    #[test]
    fn builder_clock_only_fallback_records_token_audit_events() {
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let (kernel, audit) = configured_builder_with_default_audit(Some(clock), None, None, true);
        let audit =
            audit.expect("clock-only builder should surface the fallback in-memory audit sink");

        kernel
            .issue_token(DEFAULT_PACK_ID, "spec-audit-builder-clock-only", 60)
            .expect("token issue should succeed with the clock-only in-memory audit fallback");

        let events = audit.snapshot();
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, AuditEventKind::TokenIssued { .. })),
            "expected token issuance to be recorded by the clock-only fallback in-memory audit sink"
        );
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
            kernel: &LoongKernel<StaticPolicyEngine>,
        ) -> kernel::CapabilityToken {
            kernel
                .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
                .expect("token issue should succeed via legacy helper signature")
        }

        let kernel = BootstrapBuilder::default().build();
        let token = issue_default_pack_token(&kernel);
        assert!(!token.token_id.is_empty());
    }

    #[tokio::test]
    async fn default_builder_registers_stub_embedded_harness() {
        let kernel = KernelBuilder::default().build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
            .expect("token issue should succeed");

        let dispatch = kernel
            .execute_task(
                DEFAULT_PACK_ID,
                &token,
                kernel::TaskIntent {
                    task_id: "stub-harness-task".to_owned(),
                    objective: "stub objective".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                    payload: serde_json::json!({}),
                },
            )
            .await
            .expect("default builder should dispatch through stub harness");

        assert_eq!(dispatch.adapter_route.adapter.as_deref(), Some("pi-local"));
        assert_eq!(dispatch.outcome.output["adapter"], "pi-local");
        assert_eq!(dispatch.outcome.output["task"], "stub-harness-task");
    }

    #[tokio::test]
    async fn bootstrap_builder_can_skip_default_embedded_harness_and_register_explicit_one() {
        struct MarkerHarness;

        #[async_trait::async_trait]
        impl kernel::HarnessAdapter for MarkerHarness {
            fn name(&self) -> &str {
                "pi-local"
            }

            fn kind(&self) -> HarnessKind {
                HarnessKind::EmbeddedPi
            }

            async fn execute(
                &self,
                request: kernel::HarnessRequest,
            ) -> Result<kernel::HarnessOutcome, kernel::HarnessError> {
                Ok(kernel::HarnessOutcome {
                    status: "ok".to_owned(),
                    output: serde_json::json!({
                        "adapter": "explicit-marker",
                        "task": request.task_id,
                        "objective": request.objective,
                    }),
                })
            }
        }

        let mut builder = BootstrapBuilder::default()
            .without_default_embedded_harness()
            .into_builder();
        builder.register_harness_adapter(MarkerHarness);
        let kernel = builder.build();
        let token = kernel
            .issue_token(DEFAULT_PACK_ID, "test-agent", 60)
            .expect("token issue should succeed");

        let dispatch = kernel
            .execute_task(
                DEFAULT_PACK_ID,
                &token,
                kernel::TaskIntent {
                    task_id: "marker-harness-task".to_owned(),
                    objective: "marker objective".to_owned(),
                    required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                    payload: serde_json::json!({}),
                },
            )
            .await
            .expect("explicit marker harness should dispatch");

        assert_eq!(dispatch.adapter_route.adapter.as_deref(), Some("pi-local"));
        assert_eq!(dispatch.outcome.output["adapter"], "explicit-marker");
        assert_eq!(dispatch.outcome.output["task"], "marker-harness-task");
    }
}
