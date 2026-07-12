use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::error::FsActionError;

mod sealed {
    pub trait Sealed {}
}

/// Marker for target-following path resolution.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetPath;

/// Marker for final-component no-follow path resolution.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryPath;

impl sealed::Sealed for TargetPath {}
impl sealed::Sealed for EntryPath {}

/// Sealed path-resolution semantics carried through the fs typestate chain.
///
/// This is public only because it bounds public generic action types. External
/// code cannot implement it, and normal callers use the concrete path aliases.
#[doc(hidden)]
pub trait FsPathMode: sealed::Sealed + Send + Sync + 'static {
    const NAME: &'static str;
    const RESOLVE_OPERATION: &'static str;
    const AUTHORIZE_OPERATION: &'static str;
    const FOLLOWS_FINAL_COMPONENT: bool;
}

impl FsPathMode for TargetPath {
    const NAME: &'static str = "target";
    const RESOLVE_OPERATION: &'static str = "resolve_target_path";
    const AUTHORIZE_OPERATION: &'static str = "authorize_target_path";
    const FOLLOWS_FINAL_COMPONENT: bool = true;
}

impl FsPathMode for EntryPath {
    const NAME: &'static str = "entry";
    const RESOLVE_OPERATION: &'static str = "resolve_entry_path";
    const AUTHORIZE_OPERATION: &'static str = "authorize_entry_path";
    const FOLLOWS_FINAL_COMPONENT: bool = false;
}

/// Resolved path facts produced by running a granted resolve action.
///
/// Resolution does not imply authorization. `FsPathAction` consumes this
/// unforgeable value so policy can decide whether the resolved path is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFsPath<M> {
    requested: PathBuf,
    path: PathBuf,
    _mode: std::marker::PhantomData<fn() -> M>,
}

impl<M> ResolvedFsPath<M> {
    pub(in crate::fs) fn new(requested: PathBuf, path: PathBuf) -> Self {
        Self {
            requested,
            path,
            _mode: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn requested_path(&self) -> &Path {
        &self.requested
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::fs) fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

/// Target-following resolved path facts.
pub type ResolvedPath = ResolvedFsPath<TargetPath>;

/// Final-component no-follow resolved path facts.
pub type ResolvedEntryPath = ResolvedFsPath<EntryPath>;

/// Filesystem path produced by governed path authorization.
///
/// Downstream fs actions accept this value instead of raw paths so their
/// constructors prove that resolve and path policy have already run. Only the
/// fs module can mint one.
///
/// This authorizes the path observed during resolution; it does not pin the
/// underlying inode. A descriptor-relative backend is still required to close
/// races where another actor replaces a path between authorization and use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedFsPath<M> {
    path: PathBuf,
    _mode: std::marker::PhantomData<fn() -> M>,
}

impl<M> GrantedFsPath<M> {
    pub(in crate::fs) fn new(path: PathBuf) -> Self {
        Self {
            path,
            _mode: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl<M> AsRef<Path> for GrantedFsPath<M> {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

/// Governed target-following path used by content and directory operations.
pub type GrantedPath = GrantedFsPath<TargetPath>;

/// Governed entry path used by unlink and rename operations.
pub type GrantedEntryPath = GrantedFsPath<EntryPath>;

pub(in crate::fs) fn resolve_target_path(
    path: &Path,
    resolution_root: &Path,
) -> Result<PathBuf, FsActionError> {
    if path.as_os_str().is_empty() {
        return Err(FsActionError::EmptyPath);
    }

    let resolution_root = resolve_existing_or_missing_path(resolution_root)?;
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolution_root.join(path)
    };
    resolve_existing_or_missing_path(&combined)
}

pub(in crate::fs) fn resolve_entry_path(
    path: &Path,
    resolution_root: &Path,
) -> Result<PathBuf, FsActionError> {
    if path.as_os_str().is_empty() {
        return Err(FsActionError::EmptyPath);
    }

    let resolution_root = resolve_existing_or_missing_path(resolution_root)?;
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolution_root.join(path)
    };
    let normalized = normalize_without_fs(&combined);
    let file_name = normalized
        .file_name()
        .map(std::ffi::OsStr::to_owned)
        .ok_or_else(|| FsActionError::MissingFileName {
            path: normalized.clone(),
        })?;
    let parent = normalized
        .parent()
        .ok_or_else(|| FsActionError::MissingExistingAncestor {
            path: normalized.clone(),
        })?;
    let mut resolved = resolve_existing_or_missing_path(parent)?;
    resolved.push(file_name);

    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn resolve_existing_or_missing_path(path: &Path) -> Result<PathBuf, FsActionError> {
    let normalized = normalize_without_fs(path);
    if !normalized.exists() {
        // Missing suffixes still inherit symlinks from existing ancestors.
        // Resolve that ancestor now so the returned path is the policy-visible
        // filesystem location, not a lexical path that std::fs would later
        // reinterpret at read time.
        return resolve_from_existing_ancestor(&normalized);
    }
    canonicalize_existing_path(&normalized)
}

fn resolve_from_existing_ancestor(path: &Path) -> Result<PathBuf, FsActionError> {
    let (ancestor, suffix) = split_existing_ancestor(path)?;
    let mut resolved = canonicalize_existing_path(&ancestor)?;
    for component in suffix {
        resolved.push(component);
    }
    Ok(dunce::simplified(&resolved).to_path_buf())
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, FsActionError> {
    let canonical =
        dunce::canonicalize(path).map_err(|source| FsActionError::CanonicalizePath {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), FsActionError> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(std::ffi::OsStr::to_owned) else {
            return Err(FsActionError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        suffix.push(name);

        let Some(parent) = cursor.parent() else {
            return Err(FsActionError::MissingExistingAncestor {
                path: path.to_path_buf(),
            });
        };
        cursor = parent.to_path_buf();
    }
}

// Keep local normalization here until path normalization moves into a lower
// shared crate than `loong-access`.
fn normalize_without_fs(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = parts.last() {
                    if last != ".." {
                        let _ = parts.pop();
                    } else if !has_root {
                        parts.push(OsString::from(".."));
                    }
                } else if !has_root {
                    parts.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => parts.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }

    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}
