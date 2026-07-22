//! Errors produced after orchestration has selected the legacy kernel fallback.

use loong_contracts::{KernelError, ToolPathError};
use loong_runtime::tool_plane::error::LookupError;
use thiserror::Error;

/// Failure before provider, search, or prompt metadata can be published.
///
/// Only ordinary absence may select legacy catalog metadata. Invalid identity
/// and registry corruption remain visible so an inconsistent tool surface is
/// never published to an agent.
#[derive(Debug, Error)]
pub enum ToolMetadataError {
    #[error("invalid tool catalog path `{tool_name}`: {source}")]
    InvalidPath {
        tool_name: String,
        #[source]
        source: ToolPathError,
    },
    #[error(transparent)]
    Lookup(#[from] LookupError),
}

/// Failure owned by the old `ToolCoreRequest` adapter boundary.
///
/// Typed lookup, grant, parsing, and dispatch retain `ToolInvocationError` and
/// never cross this enum.
#[derive(Debug, Error)]
pub(crate) enum LegacyToolRequestError {
    /// A legacy dispatcher was paired with a Context from another Runtime.
    #[error("legacy dispatcher belongs to another Runtime")]
    RuntimeMismatch,
    /// The legacy envelope could not be normalized into a concrete request.
    #[error("invalid tool request: {0}")]
    Input(String),
    /// Untrusted payload attempted to supply runtime-owned execution context.
    #[error("reserved tool context denied: {0}")]
    ReservedContext(String),
    /// Failure after this ingress has explicitly selected the legacy plane.
    #[error(transparent)]
    Legacy(#[from] KernelError),
}
