use thiserror::Error;

use crate::contracts::{Capability, HarnessKind};

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PackError {
    #[error("pack_id must not be empty")]
    EmptyPackId,
    #[error("domain must not be empty")]
    EmptyDomain,
    #[error("version is not valid semver: {0}")]
    InvalidVersion(String),
    #[error("pack must grant at least one capability")]
    EmptyCapabilities,
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("token {token_id} expired at {expires_at_epoch_s}")]
    ExpiredToken {
        token_id: String,
        expires_at_epoch_s: u64,
    },
    #[error("token {token_id} missing required capability {capability:?}")]
    MissingCapability {
        token_id: String,
        capability: Capability,
    },
    #[error("token pack mismatch: token pack {token_pack_id} vs runtime pack {runtime_pack_id}")]
    PackMismatch {
        token_pack_id: String,
        runtime_pack_id: String,
    },
    #[error("token {token_id} has been revoked")]
    RevokedToken { token_id: String },
    #[error("policy extension {extension} denied request: {reason}")]
    ExtensionDenied { extension: String, reason: String },
    #[error("tool call denied by policy for `{tool_name}`: {reason}")]
    ToolCallDenied { tool_name: String, reason: String },
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HarnessError {
    #[error("harness adapter not found: {0}")]
    AdapterNotFound(String),
    #[error(
        "harness adapter kind mismatch for adapter {adapter}: expected {expected:?}, actual {actual:?}"
    )]
    AdapterKindMismatch {
        adapter: String,
        expected: HarnessKind,
        actual: HarnessKind,
    },
    #[error("harness adapter execution failed: {0}")]
    Execution(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectorError {
    #[error("connector not found: {0}")]
    NotFound(String),
    #[error("core connector adapter not found: {0}")]
    CoreAdapterNotFound(String),
    #[error("connector extension not found: {0}")]
    ExtensionNotFound(String),
    #[error("no default core connector adapter is configured")]
    NoDefaultCoreAdapter,
    #[error("connector execution failed: {0}")]
    Execution(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuntimePlaneError {
    #[error("core runtime adapter not found: {0}")]
    CoreAdapterNotFound(String),
    #[error("runtime extension not found: {0}")]
    ExtensionNotFound(String),
    #[error("no default core runtime adapter is configured")]
    NoDefaultCoreAdapter,
    #[error("runtime execution failed: {0}")]
    Execution(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolPlaneError {
    #[error("core tool adapter not found: {0}")]
    CoreAdapterNotFound(String),
    #[error("tool extension not found: {0}")]
    ExtensionNotFound(String),
    #[error("no default core tool adapter is configured")]
    NoDefaultCoreAdapter,
    #[error("tool execution failed: {0}")]
    Execution(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryPlaneError {
    #[error("core memory adapter not found: {0}")]
    CoreAdapterNotFound(String),
    #[error("memory extension not found: {0}")]
    ExtensionNotFound(String),
    #[error("no default core memory adapter is configured")]
    NoDefaultCoreAdapter,
    #[error("memory execution failed: {0}")]
    Execution(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntegrationError {
    #[error("provider not found: {0}")]
    ProviderNotFound(String),
    #[error("channel not found: {0}")]
    ChannelNotFound(String),
    #[error("channel is disabled: {0}")]
    ChannelDisabled(String),
    #[error("plugin scan root does not exist: {0}")]
    PluginScanRootNotFound(String),
    #[error("failed to read plugin source file {path}: {reason}")]
    PluginFileRead { path: String, reason: String },
    #[error("invalid plugin manifest in {path}: {reason}")]
    PluginManifestParse { path: String, reason: String },
    #[error(
        "plugin manifest conflict between package {package_manifest_path} and source {source_path} on {field}: package {package_value} vs source {source_value}"
    )]
    PluginManifestConflict {
        package_manifest_path: String,
        source_path: String,
        field: String,
        package_value: String,
        source_value: String,
    },
    #[error("awareness root does not exist: {0}")]
    AwarenessRootNotFound(String),
    #[error("failed to inspect awareness file {path}: {reason}")]
    AwarenessFileRead { path: String, reason: String },
    #[error("plugin absorb failed for {plugin_id}: {reason}")]
    PluginAbsorbFailed { plugin_id: String, reason: String },
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuditError {
    #[error("audit sink failure: {0}")]
    Sink(String),
}

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KernelError {
    #[error("pack not found: {0}")]
    PackNotFound(String),
    #[error("duplicate pack id: {0}")]
    DuplicatePack(String),
    #[error("connector {connector} is not allowed by pack {pack_id}")]
    ConnectorNotAllowed { connector: String, pack_id: String },
    #[error("pack {pack_id} does not grant capability {capability:?}")]
    PackCapabilityBoundary {
        pack_id: String,
        capability: Capability,
    },
    #[error(transparent)]
    Pack(#[from] PackError),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Harness(#[from] HarnessError),
    #[error(transparent)]
    Connector(#[from] ConnectorError),
    #[error(transparent)]
    RuntimePlane(#[from] RuntimePlaneError),
    #[error(transparent)]
    ToolPlane(#[from] ToolPlaneError),
    #[error(transparent)]
    MemoryPlane(#[from] MemoryPlaneError),
    #[error(transparent)]
    Integration(#[from] IntegrationError),
    #[error(transparent)]
    Audit(#[from] AuditError),
}
