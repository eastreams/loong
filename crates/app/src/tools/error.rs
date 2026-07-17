//! Error boundary for the temporary typed-first legacy tool ingress.

use loong_contracts::KernelError;
use loong_runtime::tool_plane::{
    ToolPath,
    error::{LookupError, ToolInvocationError},
};
use thiserror::Error;

/// Distinguishes the new typed runtime path from the explicit legacy fallback.
///
/// This sum exists only because the current ingress can select either owner. It
/// preserves both sources and must disappear with the legacy tool envelope.
#[derive(Debug, Error)]
pub(crate) enum ToolRequestError {
    /// The legacy envelope could not be normalized into a concrete request.
    #[error("invalid tool request: {0}")]
    Input(String),
    /// Untrusted payload attempted to supply runtime-owned execution context.
    #[error("reserved tool context denied: {0}")]
    ReservedContext(String),
    /// The transitional app context could not derive this invocation scope.
    #[error("tool execution context could not be derived: {0}")]
    Context(String),
    /// Typed registry lookup failed for a reason other than ordinary absence.
    #[error(transparent)]
    Lookup(#[from] LookupError<ToolPath>),
    /// A catalog identity already belongs to the typed plane, so absence cannot
    /// be reinterpreted as permission to enter a legacy dispatcher.
    #[error("typed tool `{path}` is missing its runtime registration")]
    RegistryMissing { path: ToolPath },
    /// Neither the runtime registry nor the explicit legacy owner table knows
    /// this path. Migrated tools deliberately have no legacy owner to revive.
    #[error("tool_not_found: {tool_name}")]
    NotFound { tool_name: String },
    /// A looked-up typed invocation failed during grant, audit, or dispatch.
    #[error(transparent)]
    Invocation(#[from] ToolInvocationError),
    /// Typed lookup missed and the legacy owner is the app dispatcher, which
    /// this kernel-only bridge cannot call.
    #[error("legacy app tool `{tool_name}` requires the app dispatcher")]
    LegacyAppDispatch { tool_name: String },
    /// Failure after this ingress has explicitly selected the legacy plane.
    #[error(transparent)]
    Legacy(#[from] KernelError),
}
