mod read;
mod write;

use std::path::{Path, PathBuf};

pub use read::FsReadError;
use thiserror::Error;
pub use write::FsWriteError;

use crate::Facade;

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
/// It resolves raw caller paths inside the workspace root, builds typed
/// actions, asks policy for grants, and only then touches disk.
pub struct FsAccess<'a> {
    ctx: &'a Facade,
    workspace_root: PathBuf,
}

/// Why a raw path could not be resolved inside the workspace root.
#[derive(Debug, Error)]
pub enum FsPathError {
    #[error("path `{}` is outside workspace root", .0.display())]
    OutsideWorkspace(PathBuf),
    #[error("write target must include a file name: `{}`", .0.display())]
    MissingFileName(PathBuf),
    #[error("failed to canonicalize `{}`", .0.display())]
    Io(PathBuf, #[source] std::io::Error),
}

async fn resolve_existing(root: &Path, path: &Path) -> Result<PathBuf, FsPathError> {
    let joined = root.join(path);
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|source| FsPathError::Io(root.to_path_buf(), source))?;
    let canonical = tokio::fs::canonicalize(&joined)
        .await
        .map_err(|source| FsPathError::Io(joined.clone(), source))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(FsPathError::OutsideWorkspace(joined));
    }
    Ok(canonical)
}

async fn resolve_for_write(root: &Path, path: &Path) -> Result<PathBuf, FsPathError> {
    let joined = root.join(path);
    match tokio::fs::canonicalize(&joined).await {
        Ok(canonical) => {
            let canonical_root = tokio::fs::canonicalize(root)
                .await
                .map_err(|source| FsPathError::Io(root.to_path_buf(), source))?;
            if !canonical.starts_with(&canonical_root) {
                return Err(FsPathError::OutsideWorkspace(joined));
            }
            Ok(canonical)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let file_name = joined
                .file_name()
                .ok_or_else(|| FsPathError::MissingFileName(joined.clone()))?;
            let parent = joined
                .parent()
                .ok_or_else(|| FsPathError::MissingFileName(joined.clone()))?;
            let canonical_root = tokio::fs::canonicalize(root)
                .await
                .map_err(|source| FsPathError::Io(root.to_path_buf(), source))?;
            let canonical_parent = tokio::fs::canonicalize(parent)
                .await
                .map_err(|source| FsPathError::Io(parent.to_path_buf(), source))?;
            if !canonical_parent.starts_with(&canonical_root) {
                return Err(FsPathError::OutsideWorkspace(joined));
            }
            Ok(canonical_parent.join(file_name))
        }
        Err(source) => Err(FsPathError::Io(joined, source)),
    }
}

impl<'a> FsAccess<'a> {
    #[inline]
    #[must_use]
    pub fn new(ctx: &'a Facade, workspace_root: &Path) -> Self {
        Self {
            ctx,
            workspace_root: workspace_root.to_path_buf(),
        }
    }
}
