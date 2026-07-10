use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FsActionError {
    #[error("filesystem access requires at least one allowed root")]
    MissingAllowedRoot,
    #[error("filesystem path must not be empty")]
    EmptyPath,
    #[error("failed to canonicalize filesystem path {path}: {source}", path = .path.display())]
    CanonicalizePath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot resolve existing ancestor for filesystem path {path}", path = .path.display())]
    MissingExistingAncestor { path: PathBuf },
}
