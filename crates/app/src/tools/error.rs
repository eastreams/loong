//! Error boundary for the temporary typed-first legacy tool ingress.

use loong_contracts::KernelError;
use loong_runtime::tool_plane::{
    ToolPath,
    error::{LookupError, ToolInvocationError},
};
use thiserror::Error;

/// Preserves the owner of typed and legacy execution failures during migration.
///
/// This sum exists only because the current ingress can select either path. It
/// must disappear with the legacy `ToolCoreRequest` envelope rather than become
/// a shared tool error abstraction.
#[derive(Debug, Error)]
pub(crate) enum ToolRequestError {
    #[error("invalid tool request: {0}")]
    Input(String),
    #[error("reserved tool context denied: {0}")]
    ReservedContext(String),
    #[error("tool execution context could not be derived: {0}")]
    Context(String),
    #[error(transparent)]
    Lookup(#[from] LookupError<ToolPath>),
    #[error(transparent)]
    Invocation(#[from] ToolInvocationError),
    #[error(transparent)]
    Legacy(#[from] KernelError),
}
