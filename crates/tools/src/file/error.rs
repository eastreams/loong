use std::path::PathBuf;

use loong_core::{AuthorizationError, PolicyGrantError, tool::ToolFailureKind};
use loong_kernel::access::fs::FsAccessError;
use thiserror::Error;

/// Concrete execution failures shared by the builtin file tools.
///
/// Access failures remain typed so outer orchestration can distinguish policy
/// denial from backend failure without parsing display strings.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FileToolError {
    #[error("{0}")]
    Access(
        #[from]
        #[source]
        FsAccessError,
    ),
    /// File editing currently requires UTF-8 text. Supporting other encodings
    /// may require an explicit decoding policy later; this path does not guess one yet.
    #[error("failed to decode {path} as UTF-8: {source}", path = .path.display())]
    InvalidUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("{reason}")]
    ReadResponse { reason: String },
    #[error("{reason}")]
    ApplyEdit { reason: String },
}

impl FileToolError {
    /// Preserve governed Access denial as batch-local tool control flow.
    pub(crate) fn failure_kind(&self) -> ToolFailureKind {
        match self {
            Self::Access(FsAccessError::Authorization(
                AuthorizationError::PolicyGrant(
                    PolicyGrantError::MissingCapability { .. }
                    | PolicyGrantError::Denied { .. }
                    | PolicyGrantError::PermissionDenied { .. },
                )
                | AuthorizationError::MissingCapability(_)
                | AuthorizationError::Denied { .. },
            )) => ToolFailureKind::Denied,
            Self::Access(_)
            | Self::InvalidUtf8 { .. }
            | Self::ReadResponse { .. }
            | Self::ApplyEdit { .. } => ToolFailureKind::Execution,
        }
    }
}
