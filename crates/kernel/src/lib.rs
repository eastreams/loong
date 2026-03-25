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
    AuditEvent, AuditEventKind, AuditSink, ExecutionPlane, FanoutAuditSink, InMemoryAuditSink,
    JsonlAuditSink, NoopAuditSink, PlaneTier, probe_jsonl_audit_journal_runtime_ready,
};
pub use awareness::{CodebaseAwarenessConfig, CodebaseAwarenessEngine, CodebaseAwarenessSnapshot};
pub use bootstrap::{
    BootstrapPolicy, BootstrapReport, BootstrapTask, BootstrapTaskStatus, PluginBootstrapExecutor,
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
pub use kernel::{ConnectorDispatch, Kernel, KernelBuilder, KernelDispatch, LoongClawKernel};
pub use memory::{
    CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane, MemoryTier,
};
pub use pack::VerticalPackManifest;
pub use plugin::{
    PluginAbsorbReport, PluginDescriptor, PluginManifest, PluginScanReport, PluginScanner,
    PluginSetup, PluginSetupMode, PluginSourceKind,
};
pub use plugin_ir::{
    BridgeSupportMatrix, PluginActivationCandidate, PluginActivationPlan, PluginActivationStatus,
    PluginBridgeKind, PluginIR, PluginRuntimeProfile, PluginSetupReadiness,
    PluginSetupReadinessContext, PluginTranslationReport, PluginTranslator,
    evaluate_plugin_setup_requirements,
};
pub use policy::{PolicyContext, PolicyDecision, PolicyEngine, PolicyRequest, StaticPolicyEngine};
pub use policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext};
pub use runtime::{
    CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
    RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane, RuntimeTier,
};
pub use task_supervisor::TaskSupervisor;
pub use tool::{
    CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome,
    ToolExtensionRequest, ToolPlane, ToolTier,
};

pub mod test_support;

#[cfg(test)]
mod tests;
