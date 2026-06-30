//! Filesystem action and canonical path types.
//!
//! The fs access facade converts user-facing paths into canonical path values before
//! authorization. This keeps exact, prefix, and workspace policies comparing the
//! same filesystem representation instead of raw input strings.

use std::path::{Path, PathBuf};

use loong_core::error::AuthorizationError;
use loong_core::policy::grant::Granted;

#[derive(Clone, Debug)]
pub struct CanonicalPath {
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct CanonicalRoot {
    path: CanonicalPath,
}

#[derive(Clone, Debug)]
pub struct CanonicalPrefix {
    path: CanonicalPath,
}

impl CanonicalPath {
    fn from_canonical(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn existing(path: impl AsRef<Path>) -> Result<Self, AuthorizationError> {
        let canonical = std::fs::canonicalize(path).map_err(AuthorizationError::Io)?;
        Ok(Self::from_canonical(canonical))
    }

    pub(crate) fn existing_or_parent(path: impl AsRef<Path>) -> Result<Self, AuthorizationError> {
        let path = path.as_ref();
        if path.exists() {
            return Self::existing(path);
        }

        let file_name = path.file_name().ok_or_else(|| {
            AuthorizationError::Denied("write target must include a file name".into())
        })?;
        let parent = path.parent().ok_or_else(|| {
            AuthorizationError::Denied(
                "write target must have an authorized parent directory".into(),
            )
        })?;
        let canonical_parent = std::fs::canonicalize(parent).map_err(AuthorizationError::Io)?;

        Ok(Self::from_canonical(canonical_parent.join(file_name)))
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

impl CanonicalRoot {
    pub fn existing(path: impl AsRef<Path>) -> Result<Self, AuthorizationError> {
        Ok(Self {
            path: CanonicalPath::existing(path)?,
        })
    }

    pub fn contains(&self, path: &CanonicalPath) -> bool {
        path.as_path().starts_with(self.as_path())
    }

    pub fn as_path(&self) -> &Path {
        self.path.as_path()
    }
}

impl CanonicalPrefix {
    pub fn existing(path: impl AsRef<Path>) -> Result<Self, AuthorizationError> {
        Ok(Self {
            path: CanonicalPath::existing(path)?,
        })
    }

    pub fn contains(&self, path: &CanonicalPath) -> bool {
        path.as_path().starts_with(self.as_path())
    }

    pub fn as_path(&self) -> &Path {
        self.path.as_path()
    }
}

// TODO: Implement Action for these Actions

/// The parent action of all fs actions
#[derive(Clone, Debug)]
pub struct FsAction {
    pub(crate) path: CanonicalPath,
}

impl FsAction {
    pub fn new(path: CanonicalPath) -> Self {
        Self { path }
    }
}

#[derive(Clone, Debug)]
pub struct FsReadAction {
    pub(crate) path: CanonicalPath,
}

impl FsReadAction {
    pub(crate) fn new(parent: Granted<FsAction>) -> Self {
        Self {
            path: parent.into_action().path,
        }
    }
}
#[derive(Clone, Debug)]
pub struct FsWriteAction {
    pub(crate) path: CanonicalPath,
    pub(crate) content: String,
}

impl FsWriteAction {
    pub(crate) fn new(parent: Granted<FsAction>, content: String) -> Self {
        Self {
            path: parent.into_action().path,
            content,
        }
    }
}

pub(crate) fn resolve_under_authorization(
    root: &CanonicalRoot,
    path: &Path,
) -> Result<CanonicalPath, AuthorizationError> {
    let requested = candidate_path(root.as_path(), path);
    let canonical = CanonicalPath::existing(requested)?;

    ensure_under_root(root, &canonical)?;

    Ok(canonical)
}

pub(crate) fn resolve_write_under_authorization(
    root: &CanonicalRoot,
    path: &Path,
) -> Result<CanonicalPath, AuthorizationError> {
    let requested = candidate_path(root.as_path(), path);
    let canonical = CanonicalPath::existing_or_parent(requested)?;

    ensure_under_root(root, &canonical)?;

    Ok(canonical)
}

fn ensure_under_root(
    root: &CanonicalRoot,
    candidate: &CanonicalPath,
) -> Result<(), AuthorizationError> {
    if root.contains(candidate) {
        Ok(())
    } else {
        Err(AuthorizationError::OutsideWorkspace)
    }
}

fn candidate_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
