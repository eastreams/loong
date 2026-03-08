#[cfg(feature = "tool-file")]
use std::{
    fs,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-file")]
use serde_json::{json, Value};

pub(super) fn execute_file_read_tool(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-file"))]
    {
        let _ = request;
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

        let resolved = resolve_safe_file_path(target)?;
        let bytes = fs::read(&resolved)
            .map_err(|error| format!("failed to read file {}: {error}", resolved.display()))?;
        let clipped = bytes.len() > max_bytes;
        let content_slice = if clipped { &bytes[..max_bytes] } else { &bytes };

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

pub(super) fn execute_file_write_tool(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "tool-file"))]
    {
        let _ = request;
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

        let resolved = resolve_safe_file_path(target)?;
        if create_dirs {
            if let Some(parent) = resolved.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "failed to create parent directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
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

#[cfg(feature = "tool-file")]
fn resolve_safe_file_path(raw: &str) -> Result<PathBuf, String> {
    let root = std::env::var("LOONGCLAW_FILE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let root = canonicalize_or_fallback(root)?;

    let candidate = Path::new(raw);
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = normalize_without_fs_access(&combined);
    if !normalized.starts_with(&root) {
        return Err(format!(
            "file path {} escapes configured file root {}",
            normalized.display(),
            root.display()
        ));
    }
    Ok(normalized)
}

#[cfg(feature = "tool-file")]
fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
    }
    Ok(normalize_without_fs_access(&path))
}

#[cfg(feature = "tool-file")]
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
