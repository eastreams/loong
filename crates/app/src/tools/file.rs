use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-file")]
use serde_json::{Value, json};

pub(super) fn execute_file_read_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-file"))]
    {
        let _ = (request, config);
        return Err("file tool is disabled in this build (enable feature `tool-file`)".to_owned());
    }

    #[cfg(feature = "tool-file")]
    {
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| "file.read payload must be an object".to_owned())?;
        let target = payload
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "file.read requires payload.path".to_owned())?;

        let max_bytes = payload
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(1_048_576)
            .min(8 * 1_048_576) as usize;

        let resolved = resolve_safe_file_path_with_config(target, config)?;
        let bytes = fs::read(&resolved)
            .map_err(|error| format!("failed to read file {}: {error}", resolved.display()))?;
        let clipped = bytes.len() > max_bytes;
        let content_slice = if clipped {
            bytes.get(..max_bytes).unwrap_or(&bytes)
        } else {
            &bytes
        };

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "path": resolved.display().to_string(),
                "bytes": bytes.len(),
                "truncated": clipped,
                "content": String::from_utf8_lossy(content_slice).to_string(),
            }),
        })
    }
}

pub(super) fn execute_file_write_tool_with_config(
    request: ToolCoreRequest,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-file"))]
    {
        let _ = (request, config);
        return Err("file tool is disabled in this build (enable feature `tool-file`)".to_owned());
    }

    #[cfg(feature = "tool-file")]
    {
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| "file.write payload must be an object".to_owned())?;
        let target = payload
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "file.write requires payload.path".to_owned())?;
        let content = payload
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| "file.write requires payload.content".to_owned())?;
        let create_dirs = payload
            .get("create_dirs")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let resolved = resolve_safe_file_path_with_config(target, config)?;
        if create_dirs && let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create parent directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&resolved, content)
            .map_err(|error| format!("failed to write file {}: {error}", resolved.display()))?;

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "path": resolved.display().to_string(),
                "bytes_written": content.len(),
            }),
        })
    }
}

pub(super) fn resolve_safe_file_path_with_config(
    raw: &str,
    config: &super::runtime_config::ToolRuntimeConfig,
) -> Result<PathBuf, String> {
    let root = config
        .file_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let root = canonicalize_or_fallback(root)?;

    let candidate = Path::new(raw);
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = super::normalize_without_fs(&combined);
    resolve_path_within_root(&root, &normalized)
}

fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
    }
    Ok(super::normalize_without_fs(&path))
}

fn resolve_path_within_root(root: &Path, normalized: &Path) -> Result<PathBuf, String> {
    ensure_path_within_root(root, normalized)?;

    if normalized.exists() {
        let canonical = fs::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target file path {}: {error}",
                normalized.display()
            )
        })?;
        ensure_path_within_root(root, &canonical)?;
        return Ok(canonical);
    }

    let (ancestor, suffix) = split_existing_ancestor(normalized)?;
    let canonical_ancestor = fs::canonicalize(&ancestor).map_err(|error| {
        format!(
            "failed to canonicalize ancestor {}: {error}",
            ancestor.display()
        )
    })?;
    ensure_path_within_root(root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_root(root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), String> {
    if path.starts_with(root) {
        return Ok(());
    }
    Err(format!(
        "file path {} escapes configured file root {}",
        path.display(),
        root.display()
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

#[allow(dead_code)]
fn normalize_without_fs_access(path: &Path) -> PathBuf {
    let mut parts = Vec::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                parts.push(component.as_os_str().to_owned());
            }
        }
    }
    let mut normalized = PathBuf::new();
    for part in parts {
        normalized.push(part);
    }
    normalized
}

#[cfg(all(test, feature = "tool-file"))]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use loongclaw_contracts::ToolCoreRequest;
    use serde_json::json;

    use super::*;
    use crate::tools::runtime_config::ToolRuntimeConfig;

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }

    #[cfg(unix)]
    #[test]
    fn resolve_safe_file_path_rejects_symlink_escape_on_read() {
        let base = unique_temp_dir("loongclaw-file-read");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&outside).expect("create outside");

        let outside_file = outside.join("secret.txt");
        fs::write(&outside_file, "secret").expect("write outside file");
        let link = root.join("secret-link");
        assert!(create_symlink(&outside_file, &link).is_ok());

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let error =
            resolve_safe_file_path_with_config("secret-link", &config).expect_err("escape denied");

        assert!(error.contains("escapes configured file root"));
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn file_write_rejects_symlink_directory_escape() {
        let base = unique_temp_dir("loongclaw-file-write");
        let root = base.join("root");
        let outside_dir = base.join("outside-dir");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&outside_dir).expect("create outside dir");

        let link = root.join("escape");
        assert!(create_symlink(&outside_dir, &link).is_ok());

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "escape/pwned.txt",
                "content": "owned",
                "create_dirs": true
            }),
        };
        let error =
            execute_file_write_tool_with_config(request, &config).expect_err("escape denied");

        assert!(error.contains("escapes configured file root"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_write_allows_path_inside_root() {
        let base = unique_temp_dir("loongclaw-file-safe");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let config = ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "safe/note.txt",
                "content": "hello",
                "create_dirs": true
            }),
        };
        let result = execute_file_write_tool_with_config(request, &config);
        assert!(result.is_ok());

        let written = fs::read_to_string(root.join("safe/note.txt")).expect("read written file");
        assert_eq!(written, "hello");
        let _ = fs::remove_dir_all(base);
    }
}
