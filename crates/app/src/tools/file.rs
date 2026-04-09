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
        if resolved.is_dir() {
            return Err(format!(
                "path '{}' is a directory, not a file",
                resolved.display()
            ));
        }
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

pub(super) fn execute_file_edit_tool_with_config(
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
            .ok_or_else(|| "file.edit payload must be an object".to_owned())?;

        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "file.edit requires payload.path (string)".to_owned())?;
        let old_string = payload
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "file.edit requires payload.old_string (string)".to_owned())?;
        let new_string = payload
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "file.edit requires payload.new_string (string)".to_owned())?;
        let replace_all = payload
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Empty pattern matches every boundary position — reject for a well-defined edit contract.
        if old_string.is_empty() {
            return Err("edit_failed: old_string must not be empty".to_owned());
        }

        let resolved = resolve_safe_file_path_with_config(path, config)?;
        let content = fs::read_to_string(&resolved)
            .map_err(|e| format!("failed to read {}: {e}", resolved.display()))?;

        // str::matches() — literal substring, non-overlapping, left-to-right.
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Err("edit_failed: old_string not found in file".to_owned());
        }
        if match_count > 1 && !replace_all {
            return Err(format!(
                "edit_failed: old_string matches {match_count} locations; \
                 set replace_all:true to replace all occurrences"
            ));
        }

        let (updated, replacements_made) = if replace_all {
            // str::replace uses the same non-overlapping semantics as str::matches.
            let s = content.replace(old_string, new_string);
            (s, match_count)
        } else {
            // Exactly one match confirmed above.
            let s = content.replacen(old_string, new_string, 1);
            (s, 1usize)
        };

        fs::write(&resolved, updated.as_bytes())
            .map_err(|e| format!("failed to write {}: {e}", resolved.display()))?;

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "path": resolved.display().to_string(),
                "replacements_made": replacements_made,
                "bytes_written": updated.len(),
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
        let canonical = dunce::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()));
        let canonical = canonical.map(|resolved| dunce::simplified(&resolved).to_path_buf())?;
        return Ok(canonical);
    }
    Ok(super::normalize_without_fs(&path))
}

fn resolve_path_within_root(root: &Path, normalized: &Path) -> Result<PathBuf, String> {
    ensure_path_within_root(root, normalized)?;

    if normalized.exists() {
        let canonical = dunce::canonicalize(normalized).map_err(|error| {
            format!(
                "failed to canonicalize target file path {}: {error}",
                normalized.display()
            )
        })?;
        let canonical = dunce::simplified(&canonical).to_path_buf();
        ensure_path_within_root(root, &canonical)?;
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
    ensure_path_within_root(root, &canonical_ancestor)?;

    let mut reconstructed = canonical_ancestor;
    for component in suffix {
        reconstructed.push(component);
    }
    ensure_path_within_root(root, &reconstructed)?;
    Ok(reconstructed)
}

fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), String> {
    let normalized_root = dunce::simplified(root);
    let normalized_path = dunce::simplified(path);
    if normalized_path.starts_with(normalized_root) {
        return Ok(());
    }
    Err(format!(
        "policy_denied: file path {} escapes configured file root {}",
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

        assert!(error.starts_with("policy_denied: "));
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

        assert!(error.starts_with("policy_denied: "));
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

    fn make_edit_request(
        path: &str,
        old: &str,
        new: &str,
        replace_all: Option<bool>,
    ) -> ToolCoreRequest {
        let mut map = serde_json::Map::new();
        map.insert("path".into(), Value::String(path.to_owned()));
        map.insert("old_string".into(), Value::String(old.to_owned()));
        map.insert("new_string".into(), Value::String(new.to_owned()));
        if let Some(ra) = replace_all {
            map.insert("replace_all".into(), Value::Bool(ra));
        }
        ToolCoreRequest {
            tool_name: "file.edit".to_owned(),
            payload: Value::Object(map),
        }
    }

    #[test]
    fn file_edit_single_match_succeeds() {
        let base = unique_temp_dir("loongclaw-file-edit-single");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("file.txt");
        fs::write(&target, "hello world").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let result = execute_file_edit_tool_with_config(
            make_edit_request("file.txt", "hello", "hi", None),
            &config,
        );
        assert!(result.is_ok(), "unexpected error: {result:?}");
        let outcome = result.unwrap();
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["replacements_made"], 1);
        assert_eq!(fs::read_to_string(&target).unwrap(), "hi world");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_no_match_errors() {
        let base = unique_temp_dir("loongclaw-file-edit-nomatch");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("file.txt"), "hello world").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let err = execute_file_edit_tool_with_config(
            make_edit_request("file.txt", "nothere", "x", None),
            &config,
        )
        .expect_err("should fail");
        assert!(err.contains("old_string not found"), "got: {err}");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_multiple_match_without_replace_all_errors() {
        let base = unique_temp_dir("loongclaw-file-edit-multi");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("file.txt"), "a\na\n").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let err = execute_file_edit_tool_with_config(
            make_edit_request("file.txt", "a", "b", None),
            &config,
        )
        .expect_err("should fail");
        assert!(err.contains("matches 2 locations"), "got: {err}");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_replace_all_replaces_all_occurrences() {
        let base = unique_temp_dir("loongclaw-file-edit-replaceall");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("file.txt");
        fs::write(&target, "a\na\na\n").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let result = execute_file_edit_tool_with_config(
            make_edit_request("file.txt", "a", "b", Some(true)),
            &config,
        );
        assert!(result.is_ok(), "unexpected error: {result:?}");
        let outcome = result.unwrap();
        assert_eq!(outcome.payload["replacements_made"], 3);
        assert_eq!(fs::read_to_string(&target).unwrap(), "b\nb\nb\n");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_empty_old_string_errors() {
        let base = unique_temp_dir("loongclaw-file-edit-empty");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("file.txt"), "hello").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let err = execute_file_edit_tool_with_config(
            make_edit_request("file.txt", "", "x", None),
            &config,
        )
        .expect_err("should fail");
        assert!(err.contains("old_string must not be empty"), "got: {err}");
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn file_edit_rejects_path_escape() {
        let base = unique_temp_dir("loongclaw-file-edit-escape");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&outside).expect("create outside");

        let outside_file = outside.join("secret.txt");
        fs::write(&outside_file, "secret content here").expect("write outside");
        let link = root.join("escape-link");
        assert!(create_symlink(&outside_file, &link).is_ok());

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let err = execute_file_edit_tool_with_config(
            make_edit_request("escape-link", "secret", "pwned", None),
            &config,
        )
        .expect_err("escape denied");

        assert!(err.starts_with("policy_denied: "));
        assert!(err.contains("escapes configured file root"));
        let _ = fs::remove_dir_all(base);
    }
}
