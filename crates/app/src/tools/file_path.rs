use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

// Shared legacy path resolver for app tools that still need config/file-root
// based path checks before they migrate to access-backed fs actions. File tool
// execution lives in `file.rs`; this module stays feature-independent because
// provider/skills/shell helpers may still need path resolution without exposing
// the file tool surface.
pub(super) fn resolve_safe_file_path_with_config(
    raw: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    let allowed_roots = collect_allowed_roots(config)?;
    // Authorization uses the whole allowed_roots set. This root is only the
    // legacy default for resolving relative paths and formatting denial text.
    let fallback_root = allowed_roots
        .first()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let resolution_root = config
        .path_resolution_root()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback_root.clone());

    let candidate = Path::new(raw);
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        resolution_root.join(candidate)
    };
    let normalized = loong_kernel::access::fs::normalize_path_lexically(&combined);
    resolve_path_within_allowed_roots(&allowed_roots, &fallback_root, &normalized)
}

pub(super) fn resolve_safe_directory_path_with_config(
    raw: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    let resolved = resolve_safe_file_path_with_config(raw, config)?;
    let exists = resolved.exists();
    if !exists {
        let message = format!(
            "policy_denied: shell cwd {} does not exist",
            resolved.display()
        );
        return Err(message);
    }
    let is_directory = resolved.is_dir();
    if !is_directory {
        let message = format!(
            "policy_denied: shell cwd {} is not a directory",
            resolved.display()
        );
        return Err(message);
    }
    Ok(resolved)
}

pub(super) fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        let canonical = dunce::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
        let canonical = canonical.map(|resolved| dunce::simplified(&resolved).to_path_buf())?;
        return Ok(canonical);
    }
    Ok(loong_kernel::access::fs::normalize_path_lexically(&path))
}

fn collect_allowed_roots(
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<PathBuf>, String> {
    let mut raw_roots = Vec::new();

    if let Some(file_root) = config.file_root.as_ref() {
        raw_roots.push(file_root.clone());
    }

    if let Some(workspace_root) = config.workspace_root.as_ref() {
        let workspace_root_is_new = raw_roots.iter().all(|root| root != workspace_root);
        if workspace_root_is_new {
            raw_roots.push(workspace_root.clone());
        }
    }

    if raw_roots.is_empty() {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        raw_roots.push(current_dir);
    }

    raw_roots
        .into_iter()
        .map(canonicalize_or_fallback)
        .collect::<Result<Vec<_>, _>>()
}

fn resolve_path_within_allowed_roots(
    allowed_roots: &[PathBuf],
    fallback_root: &Path,
    normalized: &Path,
) -> Result<PathBuf, String> {
    if normalized.exists() {
        let canonical = dunce::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target file path {}: {error}",
                normalized.display()
            )
        })?;
        let canonical = dunce::simplified(&canonical).to_path_buf();
        ensure_path_within_allowed_roots(allowed_roots, fallback_root, &canonical)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(normalized)?;
    let canonical_ancestor = dunce::canonicalize(&ancestor).map_err(|error| {
        format!(
            "failed to canonicalize ancestor {}: {error}",
            ancestor.display()
        )
    })?;
    let canonical_ancestor = dunce::simplified(&canonical_ancestor).to_path_buf();
    ensure_path_within_allowed_roots(allowed_roots, fallback_root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_allowed_roots(allowed_roots, fallback_root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_allowed_roots(
    allowed_roots: &[PathBuf],
    fallback_root: &Path,
    path: &Path,
) -> Result<(), String> {
    let normalized_path = dunce::simplified(path);
    let path_is_allowed = allowed_roots
        .iter()
        .any(|allowed_root| normalized_path.starts_with(allowed_root));
    if path_is_allowed {
        return Ok(());
    }

    Err(format!(
        "policy_denied: file path {} escapes configured file root {}",
        path.display(),
        fallback_root.display()
    ))
}

fn split_existing_ancestor(path: &Path) -> Result<(PathBuf, Vec<OsString>), String> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();

    loop {
        if cursor.exists() {
            suffix.reverse();
            return Ok((cursor, suffix));
        }

        let Some(name) = cursor.file_name().map(|value| value.to_owned()) else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        suffix.push(name);
        let Some(parent) = cursor.parent() else {
            return Err(format!(
                "cannot resolve existing ancestor for {}",
                path.display()
            ));
        };
        cursor = parent.to_path_buf();
    }
}
