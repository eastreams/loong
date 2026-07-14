use std::path::PathBuf;

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
