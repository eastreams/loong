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
pub mod tool;

pub use architecture::{
    ArchitectureBoundaryPolicy, ArchitectureGuardReport, ArchitecturePathDecision,
    ArchitecturePathReport,
};
pub use audit::{
    AuditEvent, AuditEventKind, AuditSink, ExecutionPlane, InMemoryAuditSink, NoopAuditSink,
    PlaneTier,
};
pub use awareness::{CodebaseAwarenessConfig, CodebaseAwarenessEngine, CodebaseAwarenessSnapshot};
pub use bootstrap::{
    BootstrapPolicy, BootstrapReport, BootstrapTask, BootstrapTaskStatus, PluginBootstrapExecutor,
};
pub use clock::{Clock, FixedClock, SystemClock};
pub use connector::{
    ConnectorAdapter, ConnectorExtensionAdapter, ConnectorPlane, ConnectorRegistry, ConnectorTier,
    CoreConnectorAdapter,
};
pub use contracts::{
    Capability, CapabilityToken, ConnectorCommand, ConnectorOutcome, ExecutionRoute, HarnessKind,
    HarnessOutcome, HarnessRequest, TaskIntent,
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
pub use kernel::{ChumosKernel, ConnectorDispatch, KernelDispatch};
pub use memory::{
    CoreMemoryAdapter, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest, MemoryPlane, MemoryTier,
};
pub use pack::VerticalPackManifest;
pub use plugin::{
    PluginAbsorbReport, PluginDescriptor, PluginManifest, PluginScanReport, PluginScanner,
};
pub use plugin_ir::{
    BridgeSupportMatrix, PluginActivationCandidate, PluginActivationPlan, PluginActivationStatus,
    PluginBridgeKind, PluginIR, PluginRuntimeProfile, PluginTranslationReport, PluginTranslator,
};
pub use policy::{PolicyEngine, StaticPolicyEngine};
pub use policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext};
pub use runtime::{
    CoreRuntimeAdapter, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
    RuntimeExtensionOutcome, RuntimeExtensionRequest, RuntimePlane, RuntimeTier,
};
pub use tool::{
    CoreToolAdapter, ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome,
    ToolExtensionRequest, ToolPlane, ToolTier,
};

#[cfg(test)]
mod tests;
