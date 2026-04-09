use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-file")]
use regex::Regex;
#[cfg(feature = "tool-file")]
use serde_json::{Value, json};
#[cfg(feature = "tool-file")]
use std::io::Write as _;
#[cfg(feature = "tool-file")]
use tempfile::NamedTempFile;

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
        let overwrite = payload
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let resolved = resolve_safe_file_path_with_config(target, config)?;
        if resolved.is_dir() {
            return Err(format!(
                "path '{}' is a directory, not a file",
                resolved.display()
            ));
        }
        if create_dirs && let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create parent directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let path_is_symlink = symlink_metadata_is_symlink(&resolved);
        if path_is_symlink {
            return Err(format!(
                "policy_denied: file.write refuses to open symlink {}",
                resolved.display()
            ));
        }

        if overwrite {
            write_file_atomically(&resolved, content)?;
        } else {
            write_new_file_without_overwrite(&resolved, content)?;
        }

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
fn symlink_metadata_is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(feature = "tool-file")]
fn write_new_file_without_overwrite(path: &Path, content: &str) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    options.create_new(true);

    let mut file = options.open(path).map_err(|error| {
        let error_kind = error.kind();
        if error_kind == std::io::ErrorKind::AlreadyExists {
            return format!(
                "policy_denied: file.write requires overwrite=true for existing file {}",
                path.display()
            );
        }

        format!("failed to open file {}: {error}", path.display())
    })?;
    file.write_all(content.as_bytes())
        .map_err(|error| format!("failed to write file {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(feature = "tool-file")]
fn write_file_atomically(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_file = NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to open file {}: {error}", path.display()))?;
    temp_file
        .write_all(content.as_bytes())
        .map_err(|error| format!("failed to write file {}: {error}", path.display()))?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|error| format!("failed to write file {}: {error}", path.display()))?;
    temp_file
        .persist(path)
        .map_err(|error| format!("failed to write file {}: {}", path.display(), error.error))?;
    Ok(())
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

pub(super) fn execute_glob_search_tool_with_config(
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
            .ok_or_else(|| "glob.search payload must be an object".to_owned())?;
        let root = resolve_search_root(payload, config, "glob.search")?;
        let pattern = required_string_field(payload, "pattern", "glob.search")?;
        let max_results = optional_u64_field(payload, "max_results", 50, 1, 200, "glob.search")?;
        let include_directories = payload
            .get("include_directories")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let matcher = GlobMatcher::new(pattern)?;
        let mut matches = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);

        while let Some(directory) = queue.pop_front() {
            let mut children = read_sorted_directory_entries(&directory)?;

            while let Some(child) = children.pop() {
                let child_path = child.path();
                let relative_path = relative_path_text(&root, &child_path)?;
                let is_directory = child
                    .file_type()
                    .map_err(|error| {
                        format!("failed to inspect {}: {error}", child_path.display())
                    })?
                    .is_dir();

                if matcher.is_match(relative_path.as_str())
                    && (!is_directory || include_directories)
                {
                    matches.push(json!({
                        "path": relative_path,
                        "kind": if is_directory { "directory" } else { "file" },
                    }));
                }

                if matches.len() >= max_results {
                    return Ok(search_outcome(
                        request.tool_name,
                        root,
                        pattern,
                        max_results,
                        true,
                        matches,
                    ));
                }

                if is_directory {
                    queue.push_back(child_path);
                }
            }
        }

        Ok(search_outcome(
            request.tool_name,
            root,
            pattern,
            max_results,
            false,
            matches,
        ))
    }
}

pub(super) fn execute_content_search_tool_with_config(
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
            .ok_or_else(|| "content.search payload must be an object".to_owned())?;
        let root = resolve_search_root(payload, config, "content.search")?;
        let query = required_string_field(payload, "query", "content.search")?;
        let glob = optional_trimmed_string_field(payload.get("glob"));
        let max_results = optional_u64_field(payload, "max_results", 20, 1, 100, "content.search")?;
        let max_bytes_per_file = optional_u64_field(
            payload,
            "max_bytes_per_file",
            262_144,
            1,
            1_048_576,
            "content.search",
        )?;
        let case_sensitive = payload
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let matcher = match glob {
            Some(pattern) => Some(GlobMatcher::new(pattern)?),
            None => None,
        };
        let mut matches = Vec::new();
        let mut queue = VecDeque::from([root.clone()]);

        while let Some(directory) = queue.pop_front() {
            let mut children = read_sorted_directory_entries(&directory)?;

            while let Some(child) = children.pop() {
                let child_path = child.path();
                let file_type = child.file_type().map_err(|error| {
                    format!("failed to inspect {}: {error}", child_path.display())
                })?;

                if file_type.is_dir() {
                    queue.push_back(child_path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }

                let relative_path = relative_path_text(&root, &child_path)?;
                if let Some(glob_matcher) = matcher.as_ref()
                    && !glob_matcher.is_match(relative_path.as_str())
                {
                    continue;
                }

                let file_bytes = fs::read(&child_path)
                    .map_err(|error| format!("failed to read {}: {error}", child_path.display()))?;
                let limited_bytes = if file_bytes.len() > max_bytes_per_file {
                    file_bytes
                        .get(..max_bytes_per_file)
                        .unwrap_or(file_bytes.as_slice())
                        .to_vec()
                } else {
                    file_bytes
                };
                let file_text = String::from_utf8_lossy(&limited_bytes).to_string();
                let query_match = find_content_match(file_text.as_str(), query, case_sensitive);

                let Some((byte_start, byte_end)) = query_match else {
                    continue;
                };

                let line_info = compute_line_info(file_text.as_str(), byte_start);
                let snippet = build_snippet(file_text.as_str(), byte_start, byte_end);
                matches.push(json!({
                    "path": relative_path,
                    "line": line_info.line,
                    "column": line_info.column,
                    "match_text": &file_text[byte_start..byte_end],
                    "snippet": snippet,
                    "truncated_file": limited_bytes.len() == max_bytes_per_file,
                }));

                if matches.len() >= max_results {
                    return Ok(search_outcome(
                        request.tool_name,
                        root,
                        query,
                        max_results,
                        true,
                        matches,
                    ));
                }
            }
        }

        Ok(search_outcome(
            request.tool_name,
            root,
            query,
            max_results,
            false,
            matches,
        ))
    }
}

#[cfg(feature = "tool-file")]
fn search_outcome(
    tool_name: String,
    root: PathBuf,
    needle: &str,
    max_results: usize,
    truncated: bool,
    matches: Vec<Value>,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": tool_name,
            "root": root.display().to_string(),
            "query": needle,
            "max_results": max_results,
            "truncated": truncated,
            "match_count": matches.len(),
            "matches": matches,
        }),
    }
}

#[cfg(feature = "tool-file")]
fn resolve_search_root(
    payload: &serde_json::Map<String, Value>,
    config: &super::runtime_config::ToolRuntimeConfig,
    tool_name: &str,
) -> Result<PathBuf, String> {
    let root = optional_trimmed_string_field(payload.get("root"));

    match root {
        Some(path) => resolve_safe_file_path_with_config(path, config),
        None => config
            .file_root
            .clone()
            .map(canonicalize_or_fallback)
            .transpose()?
            .or_else(|| std::env::current_dir().ok())
            .map(|path| canonicalize_or_fallback(path).unwrap_or_else(|_| PathBuf::from(".")))
            .ok_or_else(|| format!("{tool_name} could not determine a workspace root")),
    }
}

#[cfg(feature = "tool-file")]
fn required_string_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field: &str,
    tool_name: &str,
) -> Result<&'a str, String> {
    optional_trimmed_string_field(payload.get(field))
        .ok_or_else(|| format!("{tool_name} requires payload.{field}"))
}

#[cfg(feature = "tool-file")]
fn optional_trimmed_string_field(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "tool-file")]
fn optional_u64_field(
    payload: &serde_json::Map<String, Value>,
    field: &str,
    default_value: usize,
    minimum: usize,
    maximum: usize,
    tool_name: &str,
) -> Result<usize, String> {
    let Some(value) = payload.get(field) else {
        return Ok(default_value);
    };

    let parsed_value = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field} must be an integer"))?
        as usize;

    if parsed_value < minimum || parsed_value > maximum {
        return Err(format!(
            "{tool_name} payload.{field} must be between {minimum} and {maximum}"
        ));
    }

    Ok(parsed_value)
}

#[cfg(feature = "tool-file")]
fn read_sorted_directory_entries(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read directory {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read directory {}: {error}", directory.display()))?;

    entries.sort_by_key(|left| left.path());
    entries.reverse();
    Ok(entries)
}

#[cfg(feature = "tool-file")]
fn relative_path_text(root: &Path, path: &Path) -> Result<String, String> {
    let relative_path = path.strip_prefix(root).map_err(|error| {
        format!(
            "failed to render relative path for {} from {}: {error}",
            path.display(),
            root.display()
        )
    })?;
    let relative_text = relative_path.to_string_lossy().replace('\\', "/");
    Ok(relative_text)
}

#[cfg(feature = "tool-file")]
fn find_content_match(content: &str, query: &str, case_sensitive: bool) -> Option<(usize, usize)> {
    if case_sensitive {
        let byte_start = content.find(query)?;
        let byte_end = byte_start + query.len();
        return Some((byte_start, byte_end));
    }

    let lowercase_content = content.to_lowercase();
    let lowercase_query = query.to_lowercase();
    let byte_start = lowercase_content.find(lowercase_query.as_str())?;
    let byte_end = byte_start + lowercase_query.len();
    Some((byte_start, byte_end))
}

#[cfg(feature = "tool-file")]
fn build_snippet(content: &str, byte_start: usize, byte_end: usize) -> String {
    let snippet_start = content[..byte_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let snippet_end = content[byte_end..]
        .find('\n')
        .map(|index| byte_end + index)
        .unwrap_or(content.len());
    content[snippet_start..snippet_end].trim().to_owned()
}

#[cfg(feature = "tool-file")]
fn compute_line_info(content: &str, byte_start: usize) -> LineInfo {
    let prefix = &content[..byte_start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map(|segment| segment.chars().count() + 1)
        .unwrap_or(1);
    LineInfo { line, column }
}

#[cfg(feature = "tool-file")]
struct LineInfo {
    line: usize,
    column: usize,
}

#[cfg(feature = "tool-file")]
struct GlobMatcher {
    regex: Regex,
}

#[cfg(feature = "tool-file")]
impl GlobMatcher {
    fn new(pattern: &str) -> Result<Self, String> {
        let regex_pattern = glob_pattern_to_regex(pattern);
        let regex = Regex::new(regex_pattern.as_str())
            .map_err(|error| format!("invalid glob pattern `{pattern}`: {error}"))?;
        Ok(Self { regex })
    }

    fn is_match(&self, relative_path: &str) -> bool {
        self.regex.is_match(relative_path)
    }
}

#[cfg(feature = "tool-file")]
fn glob_pattern_to_regex(pattern: &str) -> String {
    let mut regex_pattern = String::from("^");
    let mut chars = pattern.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '*' {
            let next_is_star = chars.peek() == Some(&'*');
            if next_is_star {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    regex_pattern.push_str("(?:.*/)?");
                    continue;
                }
                regex_pattern.push_str(".*");
                continue;
            }
            regex_pattern.push_str("[^/]*");
            continue;
        }

        if character == '?' {
            regex_pattern.push_str("[^/]");
            continue;
        }

        if ".+()^$|{}[]\\".contains(character) {
            regex_pattern.push('\\');
        }
        if character == '\\' {
            regex_pattern.push('/');
            continue;
        }
        regex_pattern.push(character);
    }

    regex_pattern.push('$');
    regex_pattern
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
            file_root: Some(root.clone()),
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
            file_root: Some(root.clone()),
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

    #[test]
    fn file_write_rejects_existing_file_without_overwrite_flag() {
        let base = unique_temp_dir("loongclaw-file-overwrite-denied");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let target_path = root.join("note.txt");
        fs::write(&target_path, "original").expect("seed original file");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "note.txt",
                "content": "updated",
                "create_dirs": true
            }),
        };
        let error = execute_file_write_tool_with_config(request, &config)
            .expect_err("existing file should require overwrite=true");

        assert!(
            error.contains("overwrite=true"),
            "unexpected error: {error}"
        );
        let written = fs::read_to_string(&target_path).expect("read original file");
        assert_eq!(written, "original");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_write_allows_existing_file_with_overwrite_true() {
        let base = unique_temp_dir("loongclaw-file-overwrite-allowed");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let target_path = root.join("note.txt");
        fs::write(&target_path, "original").expect("seed original file");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "note.txt",
                "content": "updated",
                "create_dirs": true,
                "overwrite": true
            }),
        };
        let outcome = execute_file_write_tool_with_config(request, &config)
            .expect("overwrite=true should allow replacing an existing file");

        assert_eq!(outcome.status, "ok");
        let written = fs::read_to_string(&target_path).expect("read updated file");
        assert_eq!(written, "updated");
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn file_write_rejects_dangling_symlink_even_with_overwrite_true() {
        let base = unique_temp_dir("loongclaw-file-overwrite-symlink");
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).expect("create root");
        fs::create_dir_all(&outside).expect("create outside");

        let dangling_target = outside.join("secret.txt");
        let link_path = root.join("dangling-link");
        create_symlink(&dangling_target, &link_path).expect("create dangling symlink");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "dangling-link",
                "content": "updated",
                "overwrite": true
            }),
        };
        let error =
            execute_file_write_tool_with_config(request, &config).expect_err("symlink denied");

        assert!(error.contains("refuses to open symlink"));
        assert!(!dangling_target.exists());
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

    #[test]
    fn glob_search_returns_workspace_relative_matches() {
        let base = unique_temp_dir("loongclaw-glob-search");
        let root = base.join("root");
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).expect("create nested root");
        fs::write(root.join("src/lib.rs"), "pub fn alpha() {}").expect("write lib");
        fs::write(nested.join("mod.rs"), "pub fn beta() {}").expect("write mod");
        fs::write(root.join("README.md"), "hello").expect("write readme");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "glob.search".to_owned(),
            payload: json!({
                "pattern": "src/**/*.rs",
                "max_results": 10
            }),
        };
        let outcome =
            execute_glob_search_tool_with_config(request, &config).expect("glob search succeeds");
        let matches = outcome.payload["matches"]
            .as_array()
            .expect("matches array");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0]["path"], "src/lib.rs");
        assert_eq!(matches[1]["path"], "src/nested/mod.rs");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn content_search_returns_line_column_and_snippet() {
        let base = unique_temp_dir("loongclaw-content-search");
        let root = base.join("root");
        let nested = root.join("src");
        fs::create_dir_all(&nested).expect("create nested root");
        fs::write(
            nested.join("main.rs"),
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .expect("write main");
        fs::write(root.join("notes.txt"), "hello from notes").expect("write notes");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "content.search".to_owned(),
            payload: json!({
                "query": "hello world",
                "glob": "src/**/*.rs",
                "max_results": 5
            }),
        };
        let outcome = execute_content_search_tool_with_config(request, &config)
            .expect("content search succeeds");
        let matches = outcome.payload["matches"]
            .as_array()
            .expect("matches array");
        let first = matches.first().expect("first match");

        assert_eq!(matches.len(), 1);
        assert_eq!(first["path"], "src/main.rs");
        assert_eq!(first["line"], 2);
        assert_eq!(first["column"], 15);
        assert_eq!(first["snippet"], "println!(\"hello world\");");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn content_search_honors_explicit_root() {
        let base = unique_temp_dir("loongclaw-content-search-root");
        let root = base.join("root");
        let include = root.join("include");
        let exclude = root.join("exclude");
        fs::create_dir_all(&include).expect("create include");
        fs::create_dir_all(&exclude).expect("create exclude");
        fs::write(include.join("a.txt"), "needle here").expect("write include");
        fs::write(exclude.join("b.txt"), "needle here too").expect("write exclude");

        let config = ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "content.search".to_owned(),
            payload: json!({
                "root": "include",
                "query": "needle"
            }),
        };
        let outcome = execute_content_search_tool_with_config(request, &config)
            .expect("content search succeeds");
        let matches = outcome.payload["matches"]
            .as_array()
            .expect("matches array");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["path"], "a.txt");
        let _ = fs::remove_dir_all(base);
    }
}
