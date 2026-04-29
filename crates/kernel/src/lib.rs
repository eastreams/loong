#![forbid(unsafe_code)]

pub mod architecture;
pub mod audit;
pub mod awareness;
pub mod bootstrap;
pub mod clock;
pub mod connector;
pub mod contracts;
pub mod errors;
pub mod harness;
pub mod integration;
pub mod kernel;
pub mod mailbox;
pub mod memory;
pub mod pack;
pub mod plugin;
pub mod plugin_ir;
pub mod policy;
pub mod policy_ext;
pub mod runtime;
pub mod task_supervisor;
pub mod tool;

pub use architecture::{
    ArchitectureBoundaryPolicy, ArchitectureGuardReport, ArchitecturePathDecision,
    ArchitecturePathReport,
};
pub use audit::{
    AuditEvent, AuditEventKind, AuditRepairOutcome, AuditRepairReport, AuditSink,
    AuditVerificationReport, ExecutionPlane, FanoutAuditSink, InMemoryAuditSink, JsonlAuditSink,
    NoopAuditSink, PlaneTier, probe_jsonl_audit_journal_runtime_ready, repair_jsonl_audit_journal,
    verify_jsonl_audit_journal,
};
pub use awareness::{CodebaseAwarenessConfig, CodebaseAwarenessEngine, CodebaseAwarenessSnapshot};
pub use bootstrap::{
    BootstrapPolicy, BootstrapReport, BootstrapTask, BootstrapTaskStatus, PluginBootstrapExecutor,
    plugin_bridge_is_high_risk_auto_apply,
};
pub use clock::{Clock, FixedClock, SystemClock};
pub use connector::{
    ConnectorExtensionAdapter, ConnectorPlane, ConnectorTier, CoreConnectorAdapter,
};
pub use contracts::{
    Capability, CapabilityToken, ConnectorCommand, ConnectorOutcome, ExecutionRoute, Fault,
    HarnessKind, HarnessOutcome, HarnessRequest, Namespace, TaskIntent, TaskState,
};
pub use errors::{
    AuditError, ConnectorError, HarnessError, IntegrationError, KernelError, MemoryPlaneError,
    PackError, PolicyError, RuntimePlaneError, ToolPlaneError,
};
pub use harness::{HarnessAdapter, HarnessBroker};
pub use integration::{
    AutoProvisionAgent, AutoProvisionRequest, ChannelConfig, IntegrationCatalog, IntegrationHotfix,
    ProviderConfig, ProviderTemplate, ProvisionAction, ProvisionPlan,
};
pub use kernel::{ConnectorDispatch, Kernel, KernelBuilder, KernelDispatch, LoongKernel};
pub use memory::{
    CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane, MemoryTier,
};
pub use pack::VerticalPackManifest;
pub use plugin::{
    CURRENT_PLUGIN_HOST_API, CURRENT_PLUGIN_MANIFEST_API_VERSION, PACKAGE_MANIFEST_FILE_NAME,
    PluginAbsorbReport, PluginCompatibility, PluginCompatibilityMode, PluginCompatibilityShim,
    PluginContractDialect, PluginDescriptor, PluginDiagnosticCode, PluginDiagnosticFinding,
    PluginDiagnosticPhase, PluginDiagnosticSeverity, PluginIdPrecedenceSelection, PluginManifest,
    PluginScanReport, PluginScanner, PluginSetup, PluginSetupMode, PluginSlotClaim, PluginSlotMode,
    PluginSourceKind, PluginTrustTier, format_plugin_provenance_summary,
    plugin_provenance_summary_for_descriptor, prefer_first_plugin_ids,
};
pub use plugin_ir::{
    BridgeSupportMatrix, GOVERNED_NATIVE_RUNTIME_EXTENSION_FAMILY,
    GOVERNED_SIDECAR_EXTENSION_TRUST_LANE, PluginActivationCandidate,
    PluginActivationInventoryEntry, PluginActivationPlan, PluginActivationStatus, PluginBridgeKind,
    PluginChannelBridgeContract, PluginChannelBridgeReadiness, PluginCompatibilityShimSupport,
    PluginIR, PluginNativeExtensionDeclarations, PluginRuntimeProfile,
    PluginRuntimeScaffoldDefaults, PluginSetupReadiness, PluginSetupReadinessContext,
    PluginTranslationReport, PluginTranslator, TRUSTED_HOST_EXTENSION_FAMILY,
    TRUSTED_HOST_EXTENSION_TRUST_LANE, TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS,
    evaluate_plugin_setup_requirements, plugin_native_extension_declarations_from_metadata,
    plugin_runtime_scaffold_defaults,
};
pub use policy::{PolicyContext, PolicyDecision, PolicyEngine, PolicyRequest, StaticPolicyEngine};
pub use policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext};
pub use runtime::{
    CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
    RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane, RuntimeTier,
};
pub use task_supervisor::TaskSupervisor;
pub use tool::{
    CoreToolAdapter, ToolConcurrencyClass, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter,
    ToolExtensionOutcome, ToolExtensionRequest, ToolPlane, ToolTier,
};

pub mod test_support;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test]
fn unknown_concurrency_class_requires_serial_execution() {
    assert!(!ToolConcurrencyClass::ReadOnly.requires_serial_execution());
    assert!(ToolConcurrencyClass::Mutating.requires_serial_execution());
    assert!(ToolConcurrencyClass::Unknown.requires_serial_execution());
    assert_eq!(ToolConcurrencyClass::Unknown.as_str(), "unknown");
}
