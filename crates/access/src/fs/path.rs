use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::error::FsActionError;

/// Canonical filesystem path accepted by fs actions.
///
/// Construction is the policy-relevant path check: it resolves relative paths
/// from the invocation root, canonicalizes existing ancestors, and rejects
/// escapes from the allowed roots, including symlink escapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPath(PathBuf);

impl CanonicalPath {
    /// Resolve a user path into a path that is safe for an fs action.
    pub fn resolve(
        path: impl AsRef<Path>,
        resolution_root: impl AsRef<Path>,
        allowed_roots: &[PathBuf],
    ) -> Result<Self, FsActionError> {
        let raw = path.as_ref();
        if raw.as_os_str().is_empty() {
            return Err(FsActionError::EmptyPath);
        }

        let allowed_roots = allowed_roots
            .iter()
            .map(|root| canonicalize_or_fallback(root.as_path()))
            .collect::<Result<Vec<_>, _>>()?;
        if allowed_roots.is_empty() {
            return Err(FsActionError::MissingAllowedRoot);
        }

        let resolution_root = canonicalize_or_fallback(resolution_root.as_ref())?;
        let combined = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            resolution_root.join(raw)
        };
        let path = resolve_path_within_allowed_roots(combined.as_path(), &allowed_roots)?;
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

fn resolve_path_within_allowed_roots(
    path: &Path,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, FsActionError> {
    let normalized = normalize_without_fs(path);

    // A configured root may not exist yet. In that case no existing ancestor
    // can be canonicalized, so the only meaningful check is lexical: the
    // normalized target must still be below that missing allowed root.
    if allowed_roots
        .iter()
        .any(|allowed_root| !allowed_root.exists() && normalized.starts_with(allowed_root))
    {
        return Ok(normalized);
    }

    if normalized.exists() {
        let canonical = canonicalize_existing_path(&normalized)?;
        ensure_path_within_allowed_roots(&canonical, allowed_roots)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(&normalized)?;
    let mut resolved = canonicalize_existing_path(&ancestor)?;
    ensure_path_within_allowed_roots(&resolved, allowed_roots)?;
    for component in suffix {
        resolved.push(component);
    }
    ensure_path_within_allowed_roots(&resolved, allowed_roots)?;
    Ok(resolved)
}

fn canonicalize_or_fallback(path: &Path) -> Result<PathBuf, FsActionError> {
    if path.exists() {
        return canonicalize_existing_path(path);
    }
    Ok(normalize_without_fs(path))
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, FsActionError> {
    let canonical =
        dunce::canonicalize(path).map_err(|source| FsActionError::CanonicalizePath {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn ensure_path_within_allowed_roots(
    path: &Path,
    allowed_roots: &[PathBuf],
) -> Result<(), FsActionError> {
    let normalized = dunce::simplified(path);
    if allowed_roots
        .iter()
        .any(|allowed_root| normalized.starts_with(allowed_root))
    {
        return Ok(());
    }

    Err(FsActionError::PathEscapesAllowedRoots {
        path: normalized.to_path_buf(),
        allowed_roots: allowed_roots.to_vec(),
    })
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
