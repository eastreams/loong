use std::{
    collections::VecDeque,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[cfg(feature = "tool-file")]
use super::runtime_events::{
    ToolFileChangeKind, ToolFileChangePreview, ToolRuntimeEvent, current_tool_runtime_event_sink,
};
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
#[cfg(feature = "tool-file")]
use regex::{Regex, RegexBuilder};
#[cfg(feature = "tool-file")]
use serde_json::{Value, json};
#[cfg(feature = "tool-file")]
use std::io::Write as _;
#[cfg(feature = "tool-file")]
use tempfile::NamedTempFile;

#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_LINES: usize = 8;
#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_CHARS: usize = 1_200;
#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_COMPARISON_CELLS: usize = 200_000;

#[cfg(feature = "tool-file")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileReadSelection {
    content: String,
    truncated: bool,
    line_start: Option<usize>,
    line_end: Option<usize>,
    total_lines: Option<usize>,
    next_offset: Option<usize>,
}

#[cfg(feature = "tool-file")]
fn optional_positive_usize_field(
    payload: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<Option<usize>, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    let raw_value = value
        .as_u64()
        .ok_or_else(|| format!("file.read payload.{field_name} must be a positive integer"))?;
    if raw_value == 0 {
        return Err(format!(
            "file.read payload.{field_name} must be a positive integer"
        ));
    }

    usize::try_from(raw_value)
        .map(Some)
        .map_err(|conversion_error| {
            format!("file.read payload.{field_name} is too large: {conversion_error}")
        })
}

#[cfg(feature = "tool-file")]
fn clip_file_read_content(content: &str, max_bytes: usize) -> FileReadSelection {
    let content_bytes = content.as_bytes();
    let truncated = content_bytes.len() > max_bytes;
    let visible_bytes = if truncated {
        content_bytes.get(..max_bytes).unwrap_or(content_bytes)
    } else {
        content_bytes
    };

    FileReadSelection {
        content: String::from_utf8_lossy(visible_bytes).to_string(),
        truncated,
        line_start: None,
        line_end: None,
        total_lines: None,
        next_offset: None,
    }
}

#[cfg(feature = "tool-file")]
fn select_file_read_content(
    file_text: &str,
    max_bytes: usize,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<FileReadSelection, String> {
    let line_window_requested = offset.is_some() || limit.is_some();
    if !line_window_requested {
        return Ok(clip_file_read_content(file_text, max_bytes));
    }

    let all_lines = file_text.split('\n').collect::<Vec<_>>();
    let total_lines = all_lines.len();
    let line_start = offset.unwrap_or(1);
    if line_start > total_lines {
        return Err(format!(
            "offset {line_start} is beyond end of file ({total_lines} lines total)"
        ));
    }

    let start_index = line_start.saturating_sub(1);
    let requested_line_count = limit.unwrap_or(total_lines.saturating_sub(start_index));
    let end_index = start_index
        .saturating_add(requested_line_count)
        .min(total_lines);
    let selected_lines = all_lines
        .get(start_index..end_index)
        .ok_or_else(|| "file.read internal line window is out of bounds".to_owned())?;
    let selected_content = selected_lines.join("\n");
    let mut selection = clip_file_read_content(selected_content.as_str(), max_bytes);
    selection.line_start = Some(line_start);
    selection.line_end = Some(end_index);
    selection.total_lines = Some(total_lines);
    selection.next_offset = (end_index < total_lines).then_some(end_index + 1);
    Ok(selection)
}

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
        let offset = optional_positive_usize_field(payload, "offset")?;
        let limit = optional_positive_usize_field(payload, "limit")?;

        let resolved = resolve_safe_file_path_with_config(target, config)?;
        if resolved.is_dir() {
            return Err(format!(
                "path '{}' is a directory, not a file",
                resolved.display()
            ));
        }
        let bytes = fs::read(&resolved)
            .map_err(|error| format!("failed to read file {}: {error}", resolved.display()))?;
        let file_text = String::from_utf8_lossy(&bytes).to_string();
        let selection = select_file_read_content(file_text.as_str(), max_bytes, offset, limit)?;

        let mut response_payload = json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "path": resolved.display().to_string(),
            "bytes": bytes.len(),
            "truncated": selection.truncated,
            "content": selection.content,
        });
        let Some(response_object) = response_payload.as_object_mut() else {
            return Err("file.read internal response payload must be an object".to_owned());
        };
        if let Some(line_start) = selection.line_start {
            response_object.insert("line_start".to_owned(), json!(line_start));
        }
        if let Some(line_end) = selection.line_end {
            response_object.insert("line_end".to_owned(), json!(line_end));
        }
        if let Some(total_lines) = selection.total_lines {
            response_object.insert("total_lines".to_owned(), json!(total_lines));
        }
        if let Some(next_offset) = selection.next_offset {
            response_object.insert("next_offset".to_owned(), json!(next_offset));
        }

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: response_payload,
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

        let existed_before_write = resolved.exists();
        let before_content =
            if existed_before_write {
                Some(fs::read_to_string(&resolved).map_err(|error| {
                    format!("failed to read file {}: {error}", resolved.display())
                })?)
            } else {
                None
            };

        if overwrite {
            write_file_atomically(&resolved, content)?;
        } else {
            write_new_file_without_overwrite(&resolved, content)?;
        }

        let change_kind = if existed_before_write {
            ToolFileChangeKind::Overwrite
        } else {
            ToolFileChangeKind::Create
        };
        emit_file_change_preview(
            resolved.as_path(),
            change_kind,
            before_content.as_deref(),
            content,
        );

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
                "tool_preflight_repairable: file.write requires overwrite=true for existing file {}",
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

#[cfg(feature = "tool-file")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactTextEditBlock {
    old_text: String,
    new_text: String,
}

#[cfg(feature = "tool-file")]
enum FileEditRequest {
    ExactBlocks(Vec<ExactTextEditBlock>),
    LegacySingle {
        old_string: String,
        new_string: String,
        replace_all: bool,
    },
}

#[cfg(feature = "tool-file")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedExactTextEditBlock<'a> {
    start: usize,
    end: usize,
    block: &'a ExactTextEditBlock,
}

#[cfg(feature = "tool-file")]
fn payload_optional_edit_string_field<'a>(
    payload: &'a serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<Option<&'a str>, String> {
    let Some(value) = payload.get(field_name) else {
        return Ok(None);
    };

    let string_value = value
        .as_str()
        .ok_or_else(|| format!("file.edit payload.{field_name} must be a string"))?;
    Ok(Some(string_value))
}

#[cfg(feature = "tool-file")]
fn exact_edit_block_field<'a>(
    block: &'a serde_json::Map<String, Value>,
    snake_case_field: &str,
    camel_case_field: &str,
) -> Option<&'a str> {
    block
        .get(snake_case_field)
        .and_then(Value::as_str)
        .or_else(|| block.get(camel_case_field).and_then(Value::as_str))
}

#[cfg(feature = "tool-file")]
fn parse_exact_edit_blocks(
    payload: &serde_json::Map<String, Value>,
) -> Result<Option<Vec<ExactTextEditBlock>>, String> {
    let Some(raw_blocks) = payload.get("edits") else {
        return Ok(None);
    };
    let blocks = raw_blocks
        .as_array()
        .ok_or_else(|| "file.edit payload.edits must be an array".to_owned())?;
    if blocks.is_empty() {
        return Err("file.edit payload.edits must contain at least one edit block".to_owned());
    }

    let mut parsed_blocks = Vec::with_capacity(blocks.len());
    for (index, raw_block) in blocks.iter().enumerate() {
        let block = raw_block
            .as_object()
            .ok_or_else(|| format!("file.edit payload.edits[{index}] must be an object"))?;
        let old_text = exact_edit_block_field(block, "old_text", "oldText")
            .ok_or_else(|| format!("file.edit payload.edits[{index}].old_text must be a string"))?;
        if old_text.is_empty() {
            return Err(format!(
                "edit_failed: edits[{index}].old_text must not be empty"
            ));
        }
        let new_text = exact_edit_block_field(block, "new_text", "newText")
            .ok_or_else(|| format!("file.edit payload.edits[{index}].new_text must be a string"))?;
        parsed_blocks.push(ExactTextEditBlock {
            old_text: old_text.to_owned(),
            new_text: new_text.to_owned(),
        });
    }

    Ok(Some(parsed_blocks))
}

#[cfg(feature = "tool-file")]
fn parse_file_edit_request(
    payload: &serde_json::Map<String, Value>,
) -> Result<FileEditRequest, String> {
    let parsed_blocks = parse_exact_edit_blocks(payload)?;
    let old_string = payload_optional_edit_string_field(payload, "old_string")?;
    let new_string = payload_optional_edit_string_field(payload, "new_string")?;
    let replace_all = payload
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mixed_modes = parsed_blocks.is_some()
        && (old_string.is_some() || new_string.is_some() || payload.contains_key("replace_all"));
    if mixed_modes {
        return Err(
            "file.edit does not allow mixing `edits` with legacy `old_string` / `new_string` fields"
                .to_owned(),
        );
    }

    if let Some(blocks) = parsed_blocks {
        return Ok(FileEditRequest::ExactBlocks(blocks));
    }

    let Some(old_string) = old_string else {
        return Err(
            "file.edit requires payload.edits, or legacy payload.old_string and payload.new_string"
                .to_owned(),
        );
    };
    let Some(new_string) = new_string else {
        return Err("file.edit requires payload.new_string (string)".to_owned());
    };
    if old_string.is_empty() {
        return Err("edit_failed: old_string must not be empty".to_owned());
    }

    Ok(FileEditRequest::LegacySingle {
        old_string: old_string.to_owned(),
        new_string: new_string.to_owned(),
        replace_all,
    })
}

#[cfg(feature = "tool-file")]
fn locate_exact_edit_block<'a>(
    content: &str,
    block: &'a ExactTextEditBlock,
    index: usize,
) -> Result<LocatedExactTextEditBlock<'a>, String> {
    let match_offsets = content
        .match_indices(block.old_text.as_str())
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    if match_offsets.is_empty() {
        return Err(format!(
            "edit_failed: edits[{index}].old_text not found in file"
        ));
    }
    if match_offsets.len() > 1 {
        return Err(format!(
            "edit_failed: edits[{index}].old_text matches {} locations; each edit block must match uniquely in the original file",
            match_offsets.len()
        ));
    }

    let start = match_offsets
        .first()
        .copied()
        .ok_or_else(|| format!("edit_failed: edits[{index}].old_text not found in file"))?;
    let end = start.saturating_add(block.old_text.len());
    Ok(LocatedExactTextEditBlock { start, end, block })
}

#[cfg(feature = "tool-file")]
fn apply_exact_edit_blocks(
    content: &str,
    blocks: &[ExactTextEditBlock],
) -> Result<(String, usize), String> {
    let mut located_blocks = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let located_block = locate_exact_edit_block(content, block, index)?;
        located_blocks.push(located_block);
    }
    located_blocks.sort_by_key(|left| left.start);

    for window in located_blocks.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if left.end > right.start {
            return Err(
                "edit_failed: edit blocks overlap in the original file; merge nested or overlapping edits into one block"
                    .to_owned(),
            );
        }
    }

    let mut updated = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for located_block in &located_blocks {
        updated.push_str(&content[cursor..located_block.start]);
        updated.push_str(located_block.block.new_text.as_str());
        cursor = located_block.end;
    }
    updated.push_str(&content[cursor..]);

    Ok((updated, located_blocks.len()))
}

#[cfg(feature = "tool-file")]
fn apply_legacy_single_edit(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
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
        (content.replace(old_string, new_string), match_count)
    } else {
        (content.replacen(old_string, new_string, 1), 1usize)
    };
    Ok((updated, replacements_made))
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
        let edit_request = parse_file_edit_request(payload)?;

        let resolved = resolve_safe_file_path_with_config(path, config)?;
        let content = fs::read_to_string(&resolved)
            .map_err(|e| format!("failed to read {}: {e}", resolved.display()))?;

        let (updated, replacements_made) = match &edit_request {
            FileEditRequest::ExactBlocks(blocks) => {
                apply_exact_edit_blocks(content.as_str(), blocks)
            }
            FileEditRequest::LegacySingle {
                old_string,
                new_string,
                replace_all,
            } => apply_legacy_single_edit(
                content.as_str(),
                old_string.as_str(),
                new_string.as_str(),
                *replace_all,
            ),
        }?;

        fs::write(&resolved, updated.as_bytes())
            .map_err(|e| format!("failed to write {}: {e}", resolved.display()))?;
        emit_file_change_preview(
            resolved.as_path(),
            ToolFileChangeKind::Edit,
            Some(content.as_str()),
            updated.as_str(),
        );

        let edit_blocks_applied = match &edit_request {
            FileEditRequest::ExactBlocks(blocks) => Some(blocks.len()),
            FileEditRequest::LegacySingle { .. } => None,
        };
        let mut response_payload = json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "path": resolved.display().to_string(),
            "replacements_made": replacements_made,
            "bytes_written": updated.len(),
        });
        if let Some(edit_blocks_applied) = edit_blocks_applied
            && let Some(response_object) = response_payload.as_object_mut()
        {
            response_object.insert("edit_blocks_applied".to_owned(), json!(edit_blocks_applied));
        }

        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: response_payload,
        })
    }
}

#[cfg(feature = "tool-file")]
fn emit_file_change_preview(
    path: &Path,
    kind: ToolFileChangeKind,
    before: Option<&str>,
    after: &str,
) {
    let runtime_event_sink = current_tool_runtime_event_sink();
    let Some(sink) = runtime_event_sink.as_ref() else {
        return;
    };

    let preview = build_file_change_preview(path, kind, before, after);
    let event = ToolRuntimeEvent::FileChangePreview(preview);
    sink.emit(event);
}

#[cfg(feature = "tool-file")]
fn build_file_change_preview(
    path: &Path,
    kind: ToolFileChangeKind,
    before: Option<&str>,
    after: &str,
) -> ToolFileChangePreview {
    let before_lines = before.map(split_file_preview_lines).unwrap_or_default();
    let after_lines = split_file_preview_lines(after);
    let (added_lines, removed_lines, preview) =
        summarize_file_change_preview(before_lines.as_slice(), after_lines.as_slice());
    let path_display = path.display().to_string();

    ToolFileChangePreview {
        path: path_display,
        kind,
        added_lines,
        removed_lines,
        preview,
    }
}

#[cfg(feature = "tool-file")]
fn split_file_preview_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    text.lines().map(str::to_owned).collect()
}

#[cfg(feature = "tool-file")]
fn summarize_file_change_preview(
    before_lines: &[String],
    after_lines: &[String],
) -> (usize, usize, Option<String>) {
    let comparison_cells = before_lines.len().saturating_mul(after_lines.len());
    let can_use_precise_diff = comparison_cells <= FILE_CHANGE_PREVIEW_MAX_COMPARISON_CELLS;

    if can_use_precise_diff {
        let operations = build_line_diff_operations(before_lines, after_lines);
        let added_lines = count_insert_operations(operations.as_slice());
        let removed_lines = count_delete_operations(operations.as_slice());
        let preview = build_file_change_preview_text_from_operations(operations.as_slice());

        return (added_lines, removed_lines, preview);
    }

    summarize_file_change_preview_with_boundary_fallback(before_lines, after_lines)
}

#[cfg(feature = "tool-file")]
fn summarize_file_change_preview_with_boundary_fallback(
    before_lines: &[String],
    after_lines: &[String],
) -> (usize, usize, Option<String>) {
    let common_prefix_len = shared_prefix_line_count(before_lines, after_lines);
    let common_suffix_len = shared_suffix_line_count(before_lines, after_lines, common_prefix_len);
    let removed_end = before_lines.len().saturating_sub(common_suffix_len);
    let added_end = after_lines.len().saturating_sub(common_suffix_len);
    let removed_slice = before_lines
        .get(common_prefix_len..removed_end)
        .unwrap_or(&[]);
    let added_slice = after_lines.get(common_prefix_len..added_end).unwrap_or(&[]);
    let removed_lines = removed_slice.len();
    let added_lines = added_slice.len();

    let preview = build_file_change_preview_text_with_boundary_fallback(
        common_prefix_len,
        removed_slice,
        added_slice,
    );

    (added_lines, removed_lines, preview)
}

#[cfg(feature = "tool-file")]
fn shared_prefix_line_count(before_lines: &[String], after_lines: &[String]) -> usize {
    let max_prefix_len = before_lines.len().min(after_lines.len());
    let mut prefix_len = 0_usize;

    while prefix_len < max_prefix_len {
        let Some(before_line) = before_lines.get(prefix_len) else {
            break;
        };
        let Some(after_line) = after_lines.get(prefix_len) else {
            break;
        };
        if before_line != after_line {
            break;
        }
        prefix_len = prefix_len.saturating_add(1);
    }

    prefix_len
}

#[cfg(feature = "tool-file")]
fn shared_suffix_line_count(
    before_lines: &[String],
    after_lines: &[String],
    common_prefix_len: usize,
) -> usize {
    let before_remaining = before_lines.len().saturating_sub(common_prefix_len);
    let after_remaining = after_lines.len().saturating_sub(common_prefix_len);
    let max_suffix_len = before_remaining.min(after_remaining);
    let mut suffix_len = 0_usize;

    while suffix_len < max_suffix_len {
        let before_index = before_lines.len().saturating_sub(suffix_len + 1);
        let after_index = after_lines.len().saturating_sub(suffix_len + 1);
        let Some(before_line) = before_lines.get(before_index) else {
            break;
        };
        let Some(after_line) = after_lines.get(after_index) else {
            break;
        };
        if before_line != after_line {
            break;
        }
        suffix_len = suffix_len.saturating_add(1);
    }

    suffix_len
}

#[cfg(feature = "tool-file")]
fn build_file_change_preview_text_with_boundary_fallback(
    common_prefix_len: usize,
    removed_slice: &[String],
    added_slice: &[String],
) -> Option<String> {
    if removed_slice.is_empty() && added_slice.is_empty() {
        return None;
    }

    let removed_len = removed_slice.len();
    let added_len = added_slice.len();
    let hunk_start = common_prefix_len.saturating_add(1);
    let mut preview_lines = Vec::new();
    let hunk_header = format!("@@ -{hunk_start},{removed_len} +{hunk_start},{added_len} @@");
    preview_lines.push(hunk_header);

    let mut emitted_preview_lines = 0_usize;
    let mut omitted_preview_lines = 0_usize;

    for removed_line in removed_slice {
        let can_emit_line = emitted_preview_lines < FILE_CHANGE_PREVIEW_MAX_LINES;
        if can_emit_line {
            let preview_line = format!("-{removed_line}");
            preview_lines.push(preview_line);
            emitted_preview_lines = emitted_preview_lines.saturating_add(1);
        } else {
            omitted_preview_lines = omitted_preview_lines.saturating_add(1);
        }
    }

    for added_line in added_slice {
        let can_emit_line = emitted_preview_lines < FILE_CHANGE_PREVIEW_MAX_LINES;
        if can_emit_line {
            let preview_line = format!("+{added_line}");
            preview_lines.push(preview_line);
            emitted_preview_lines = emitted_preview_lines.saturating_add(1);
        } else {
            omitted_preview_lines = omitted_preview_lines.saturating_add(1);
        }
    }

    if omitted_preview_lines > 0 {
        let omitted_line = format!("… {omitted_preview_lines} more changed line(s)");
        preview_lines.push(omitted_line);
    }

    let preview_text = preview_lines.join("\n");
    let preview_char_count = preview_text.chars().count();
    if preview_char_count <= FILE_CHANGE_PREVIEW_MAX_CHARS {
        return Some(preview_text);
    }

    let retained_char_count = FILE_CHANGE_PREVIEW_MAX_CHARS.saturating_sub(1);
    let truncated_tail = preview_text
        .chars()
        .rev()
        .take(retained_char_count)
        .collect::<Vec<_>>();
    let truncated_tail = truncated_tail.into_iter().rev().collect::<String>();
    let truncated_preview = format!("…{truncated_tail}");
    Some(truncated_preview)
}

#[cfg(feature = "tool-file")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineDiffKind {
    Equal,
    Delete,
    Insert,
}

#[cfg(feature = "tool-file")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct LineDiffOperation {
    kind: LineDiffKind,
    line: String,
}

#[cfg(feature = "tool-file")]
#[derive(Default)]
struct FileChangePreviewHunkBuilder {
    old_start: usize,
    new_start: usize,
    old_len: usize,
    new_len: usize,
    lines: Vec<String>,
}

#[cfg(feature = "tool-file")]
fn build_line_diff_operations(
    before_lines: &[String],
    after_lines: &[String],
) -> Vec<LineDiffOperation> {
    let row_count = before_lines.len().saturating_add(1);
    let column_count = after_lines.len().saturating_add(1);
    let matrix_len = row_count.saturating_mul(column_count);
    let mut matrix = vec![0_usize; matrix_len];

    let mut before_index = before_lines.len();
    while before_index > 0 {
        before_index = before_index.saturating_sub(1);

        let mut after_index = after_lines.len();
        while after_index > 0 {
            after_index = after_index.saturating_sub(1);

            let matrix_index = before_index.saturating_mul(column_count) + after_index;
            let diagonal_index = (before_index.saturating_add(1)).saturating_mul(column_count)
                + after_index.saturating_add(1);
            let down_index =
                (before_index.saturating_add(1)).saturating_mul(column_count) + after_index;
            let right_index =
                before_index.saturating_mul(column_count) + after_index.saturating_add(1);
            let Some(before_line) = before_lines.get(before_index) else {
                continue;
            };
            let Some(after_line) = after_lines.get(after_index) else {
                continue;
            };

            if before_line == after_line {
                let diagonal_value = *matrix.get(diagonal_index).unwrap_or(&0);
                let next_value = diagonal_value.saturating_add(1);
                if let Some(cell) = matrix.get_mut(matrix_index) {
                    *cell = next_value;
                }
                continue;
            }

            let down_value = *matrix.get(down_index).unwrap_or(&0);
            let right_value = *matrix.get(right_index).unwrap_or(&0);
            let next_value = down_value.max(right_value);
            if let Some(cell) = matrix.get_mut(matrix_index) {
                *cell = next_value;
            }
        }
    }

    let mut operations = Vec::new();
    let mut before_cursor = 0_usize;
    let mut after_cursor = 0_usize;
    while before_cursor < before_lines.len() && after_cursor < after_lines.len() {
        let Some(before_line) = before_lines.get(before_cursor) else {
            break;
        };
        let Some(after_line) = after_lines.get(after_cursor) else {
            break;
        };

        if before_line == after_line {
            let operation = LineDiffOperation {
                kind: LineDiffKind::Equal,
                line: before_line.clone(),
            };
            operations.push(operation);
            before_cursor = before_cursor.saturating_add(1);
            after_cursor = after_cursor.saturating_add(1);
            continue;
        }

        let down_index =
            (before_cursor.saturating_add(1)).saturating_mul(column_count) + after_cursor;
        let right_index =
            before_cursor.saturating_mul(column_count) + after_cursor.saturating_add(1);
        let down_value = *matrix.get(down_index).unwrap_or(&0);
        let right_value = *matrix.get(right_index).unwrap_or(&0);

        if down_value >= right_value {
            let operation = LineDiffOperation {
                kind: LineDiffKind::Delete,
                line: before_line.clone(),
            };
            operations.push(operation);
            before_cursor = before_cursor.saturating_add(1);
            continue;
        }

        let operation = LineDiffOperation {
            kind: LineDiffKind::Insert,
            line: after_line.clone(),
        };
        operations.push(operation);
        after_cursor = after_cursor.saturating_add(1);
    }

    while before_cursor < before_lines.len() {
        let Some(before_line) = before_lines.get(before_cursor) else {
            break;
        };
        let operation = LineDiffOperation {
            kind: LineDiffKind::Delete,
            line: before_line.clone(),
        };
        operations.push(operation);
        before_cursor = before_cursor.saturating_add(1);
    }

    while after_cursor < after_lines.len() {
        let Some(after_line) = after_lines.get(after_cursor) else {
            break;
        };
        let operation = LineDiffOperation {
            kind: LineDiffKind::Insert,
            line: after_line.clone(),
        };
        operations.push(operation);
        after_cursor = after_cursor.saturating_add(1);
    }

    operations
}

#[cfg(feature = "tool-file")]
fn count_insert_operations(operations: &[LineDiffOperation]) -> usize {
    let mut count = 0_usize;
    for operation in operations {
        if operation.kind == LineDiffKind::Insert {
            count = count.saturating_add(1);
        }
    }
    count
}

#[cfg(feature = "tool-file")]
fn count_delete_operations(operations: &[LineDiffOperation]) -> usize {
    let mut count = 0_usize;
    for operation in operations {
        if operation.kind == LineDiffKind::Delete {
            count = count.saturating_add(1);
        }
    }
    count
}

#[cfg(feature = "tool-file")]
fn build_file_change_preview_text_from_operations(
    operations: &[LineDiffOperation],
) -> Option<String> {
    let has_change = operations
        .iter()
        .any(|operation| operation.kind != LineDiffKind::Equal);
    if !has_change {
        return None;
    }

    let mut preview_lines = Vec::new();
    let mut emitted_preview_lines = 0_usize;
    let mut omitted_preview_lines = 0_usize;
    let mut current_hunk = None::<FileChangePreviewHunkBuilder>;
    let mut old_line_number = 1_usize;
    let mut new_line_number = 1_usize;

    for operation in operations {
        if operation.kind == LineDiffKind::Equal {
            finalize_file_change_preview_hunk(
                &mut preview_lines,
                &mut emitted_preview_lines,
                &mut omitted_preview_lines,
                &mut current_hunk,
            );
            old_line_number = old_line_number.saturating_add(1);
            new_line_number = new_line_number.saturating_add(1);
            continue;
        }

        if current_hunk.is_none() {
            let hunk = FileChangePreviewHunkBuilder {
                old_start: old_line_number,
                new_start: new_line_number,
                old_len: 0,
                new_len: 0,
                lines: Vec::new(),
            };
            current_hunk = Some(hunk);
        }

        let Some(hunk) = current_hunk.as_mut() else {
            continue;
        };
        let (line_prefix, advance_old, advance_new) = match operation.kind {
            LineDiffKind::Delete => {
                hunk.old_len = hunk.old_len.saturating_add(1);
                ("-", true, false)
            }
            LineDiffKind::Insert => {
                hunk.new_len = hunk.new_len.saturating_add(1);
                ("+", false, true)
            }
            LineDiffKind::Equal => continue,
        };
        let preview_line = format!("{line_prefix}{}", operation.line);
        hunk.lines.push(preview_line);

        if advance_old {
            old_line_number = old_line_number.saturating_add(1);
        }
        if advance_new {
            new_line_number = new_line_number.saturating_add(1);
        }
    }

    finalize_file_change_preview_hunk(
        &mut preview_lines,
        &mut emitted_preview_lines,
        &mut omitted_preview_lines,
        &mut current_hunk,
    );

    if omitted_preview_lines > 0 {
        let omitted_line = format!("… {omitted_preview_lines} more changed line(s)");
        preview_lines.push(omitted_line);
    }

    let preview_text = preview_lines.join("\n");
    let preview_char_count = preview_text.chars().count();
    if preview_char_count <= FILE_CHANGE_PREVIEW_MAX_CHARS {
        return Some(preview_text);
    }

    let retained_char_count = FILE_CHANGE_PREVIEW_MAX_CHARS.saturating_sub(1);
    let truncated_tail = preview_text
        .chars()
        .rev()
        .take(retained_char_count)
        .collect::<Vec<_>>();
    let truncated_tail = truncated_tail.into_iter().rev().collect::<String>();
    let truncated_preview = format!("…{truncated_tail}");
    Some(truncated_preview)
}

#[cfg(feature = "tool-file")]
fn finalize_file_change_preview_hunk(
    preview_lines: &mut Vec<String>,
    emitted_preview_lines: &mut usize,
    omitted_preview_lines: &mut usize,
    current_hunk: &mut Option<FileChangePreviewHunkBuilder>,
) {
    let Some(hunk) = current_hunk.take() else {
        return;
    };

    let header = format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len,
    );
    preview_lines.push(header);

    for line in hunk.lines {
        let can_emit_line = *emitted_preview_lines < FILE_CHANGE_PREVIEW_MAX_LINES;
        if can_emit_line {
            preview_lines.push(line);
            *emitted_preview_lines = emitted_preview_lines.saturating_add(1);
        } else {
            *omitted_preview_lines = omitted_preview_lines.saturating_add(1);
        }
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
                let file_was_truncated = file_bytes.len() > max_bytes_per_file;
                let limited_bytes = if file_was_truncated {
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
                    "truncated_file": file_was_truncated,
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

    let parsed_value_u64 = value
        .as_u64()
        .ok_or_else(|| format!("{tool_name} payload.{field} must be an integer"))?;
    let parsed_value = usize::try_from(parsed_value_u64)
        .map_err(|error| format!("{tool_name} payload.{field} is out of range: {error}"))?;

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

    let escaped_query = regex::escape(query);
    let mut regex_builder = RegexBuilder::new(escaped_query.as_str());
    regex_builder.case_insensitive(true);
    let regex = regex_builder.build().ok()?;
    let matched_range = regex.find(content)?;
    let byte_start = matched_range.start();
    let byte_end = matched_range.end();
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

    ensure_path_within_root(root, normalized)?;

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

#[cfg(all(test, feature = "tool-file"))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use loong_contracts::ToolCoreRequest;
    use serde_json::json;

    use super::*;
    use crate::tools::runtime_config::ToolRuntimeConfig;
    use crate::tools::runtime_events::{
        ToolFileChangeKind, ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
    };

    #[derive(Default)]
    struct RecordingRuntimeSink {
        events: Mutex<Vec<ToolRuntimeEvent>>,
    }

    fn lock_runtime_events(
        sink: &RecordingRuntimeSink,
    ) -> std::sync::MutexGuard<'_, Vec<ToolRuntimeEvent>> {
        match sink.events.lock() {
            Ok(events) => events,
            Err(poisoned_events) => poisoned_events.into_inner(),
        }
    }

    impl ToolRuntimeEventSink for RecordingRuntimeSink {
        fn emit(&self, event: ToolRuntimeEvent) {
            let mut events = lock_runtime_events(self);
            events.push(event);
        }
    }

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
        let base = unique_temp_dir("loong-file-read");
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

    #[test]
    fn file_read_supports_line_window_pagination() {
        let base = unique_temp_dir("loong-file-read-window");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma\ndelta").expect("write fixture");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "notes.txt",
                "offset": 2,
                "limit": 2
            }),
        };

        let outcome = execute_file_read_tool_with_config(request, &config)
            .expect("file.read window should succeed");

        assert_eq!(outcome.payload["content"], json!("beta\ngamma"));
        assert_eq!(outcome.payload["line_start"], json!(2));
        assert_eq!(outcome.payload["line_end"], json!(3));
        assert_eq!(outcome.payload["total_lines"], json!(4));
        assert_eq!(outcome.payload["next_offset"], json!(4));
        assert_eq!(outcome.payload["truncated"], json!(false));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_read_rejects_line_offset_beyond_end_of_file() {
        let base = unique_temp_dir("loong-file-read-window-bounds");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("notes.txt"), "alpha\nbeta").expect("write fixture");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "notes.txt",
                "offset": 3
            }),
        };

        let error = execute_file_read_tool_with_config(request, &config)
            .expect_err("out-of-bounds file.read window should fail");

        assert!(error.contains("offset 3 is beyond end of file (2 lines total)"));
        let _ = fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn file_write_rejects_symlink_directory_escape() {
        let base = unique_temp_dir("loong-file-write");
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
        let base = unique_temp_dir("loong-file-safe");
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
    fn file_write_emits_create_change_preview_event() {
        let base = unique_temp_dir("loong-file-write-preview");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "preview.txt",
                "content": "alpha\nbeta\n",
                "create_dirs": true
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_file_write_tool_with_config(request, &config)
        }));
        let outcome = outcome.expect("file.write should succeed");
        let events = lock_runtime_events(&sink);
        let preview = events.iter().find_map(|event| {
            if let ToolRuntimeEvent::FileChangePreview(preview) = event {
                return Some(preview);
            }

            None
        });
        let preview = preview.expect("file.write should emit change preview");

        assert_eq!(outcome.status, "ok");
        assert_eq!(preview.kind, ToolFileChangeKind::Create);
        assert_eq!(preview.added_lines, 2);
        assert_eq!(preview.removed_lines, 0);
        assert!(preview.path.ends_with("preview.txt"));
    }

    #[test]
    fn file_write_emits_overwrite_change_preview_event() {
        let base = unique_temp_dir("loong-file-write-overwrite-preview");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("preview.txt");
        fs::write(&target, "old line\nshared\n").expect("seed original file");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "file.write".to_owned(),
            payload: json!({
                "path": "preview.txt",
                "content": "new line\nshared\nextra\n",
                "overwrite": true
            }),
        };
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_file_write_tool_with_config(request, &config)
        }));
        let outcome = outcome.expect("file.write overwrite should succeed");
        let events = lock_runtime_events(&sink);
        let preview = events.iter().find_map(|event| {
            if let ToolRuntimeEvent::FileChangePreview(preview) = event {
                return Some(preview);
            }

            None
        });
        let preview = preview.expect("file.write overwrite should emit change preview");
        let preview_text = preview.preview.as_deref().unwrap_or_default();

        assert_eq!(outcome.status, "ok");
        assert_eq!(preview.kind, ToolFileChangeKind::Overwrite);
        assert_eq!(preview.added_lines, 2);
        assert_eq!(preview.removed_lines, 1);
        assert!(preview_text.contains("-old line"));
        assert!(preview_text.contains("+new line"));
        assert!(preview_text.contains("+extra"));
    }

    #[test]
    fn file_write_rejects_existing_file_without_overwrite_flag() {
        let base = unique_temp_dir("loong-file-overwrite-denied");
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
        let base = unique_temp_dir("loong-file-overwrite-allowed");
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
        let base = unique_temp_dir("loong-file-overwrite-symlink");
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

    fn make_edit_blocks_request(path: &str, edits: &[(&str, &str)]) -> ToolCoreRequest {
        let edit_blocks = edits
            .iter()
            .map(|(old, new)| {
                json!({
                    "old_text": old,
                    "new_text": new,
                })
            })
            .collect::<Vec<_>>();
        ToolCoreRequest {
            tool_name: "file.edit".to_owned(),
            payload: json!({
                "path": path,
                "edits": edit_blocks,
            }),
        }
    }

    fn make_camel_case_edit_blocks_request(path: &str, edits: &[(&str, &str)]) -> ToolCoreRequest {
        let edit_blocks = edits
            .iter()
            .map(|(old, new)| {
                json!({
                    "oldText": old,
                    "newText": new,
                })
            })
            .collect::<Vec<_>>();
        ToolCoreRequest {
            tool_name: "file.edit".to_owned(),
            payload: json!({
                "path": path,
                "edits": edit_blocks,
            }),
        }
    }

    #[test]
    fn file_edit_single_match_succeeds() {
        let base = unique_temp_dir("loong-file-edit-single");
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
        let base = unique_temp_dir("loong-file-edit-nomatch");
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
        let base = unique_temp_dir("loong-file-edit-multi");
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
        let base = unique_temp_dir("loong-file-edit-replaceall");
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
    fn file_edit_emits_change_preview_event() {
        let base = unique_temp_dir("loong-file-edit-preview");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("file.txt");
        fs::write(&target, "old line\nshared\n").expect("write original file");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = make_edit_request("file.txt", "old line", "new line", None);
        let sink = Arc::new(RecordingRuntimeSink::default());
        let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");

        let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
            execute_file_edit_tool_with_config(request, &config)
        }));
        let outcome = outcome.expect("file.edit should succeed");
        let events = lock_runtime_events(&sink);
        let preview = events.iter().find_map(|event| {
            if let ToolRuntimeEvent::FileChangePreview(preview) = event {
                return Some(preview);
            }

            None
        });
        let preview = preview.expect("file.edit should emit change preview");
        let preview_text = preview.preview.as_deref().unwrap_or_default();

        assert_eq!(outcome.status, "ok");
        assert_eq!(preview.kind, ToolFileChangeKind::Edit);
        assert_eq!(preview.added_lines, 1);
        assert_eq!(preview.removed_lines, 1);
        assert!(preview_text.contains("-old line"));
        assert!(preview_text.contains("+new line"));
    }

    #[test]
    fn file_edit_exact_edit_blocks_apply_multiple_replacements() {
        let base = unique_temp_dir("loong-file-edit-blocks");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("file.txt");
        fs::write(&target, "alpha\nbeta\ngamma\n").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let result = execute_file_edit_tool_with_config(
            make_edit_blocks_request("file.txt", &[("alpha", "ALPHA"), ("gamma", "GAMMA")]),
            &config,
        );
        assert!(result.is_ok(), "unexpected error: {result:?}");
        let outcome = result.unwrap();
        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["replacements_made"], 2);
        assert_eq!(outcome.payload["edit_blocks_applied"], 2);
        assert_eq!(fs::read_to_string(&target).unwrap(), "ALPHA\nbeta\nGAMMA\n");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_exact_edit_blocks_reject_non_unique_matches() {
        let base = unique_temp_dir("loong-file-edit-blocks-non-unique");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("file.txt"), "dup\ndup\n").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let err = execute_file_edit_tool_with_config(
            make_edit_blocks_request("file.txt", &[("dup", "only once")]),
            &config,
        )
        .expect_err("non-unique block should fail");
        assert!(err.contains("matches 2 locations"), "got: {err}");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn file_edit_exact_edit_blocks_accept_camel_case_aliases() {
        let base = unique_temp_dir("loong-file-edit-blocks-camel");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        let target = root.join("file.txt");
        fs::write(&target, "hello world").expect("write");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let result = execute_file_edit_tool_with_config(
            make_camel_case_edit_blocks_request("file.txt", &[("hello", "hi")]),
            &config,
        );
        assert!(result.is_ok(), "unexpected error: {result:?}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "hi world");
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn summarize_file_change_preview_preserves_shared_middle_lines_when_appending_tail() {
        let before_lines = vec!["old line".to_owned(), "shared".to_owned()];
        let after_lines = vec![
            "new line".to_owned(),
            "shared".to_owned(),
            "extra".to_owned(),
        ];

        let (added_lines, removed_lines, preview) =
            summarize_file_change_preview(before_lines.as_slice(), after_lines.as_slice());
        let preview = preview.expect("preview should exist");

        assert_eq!(added_lines, 2);
        assert_eq!(removed_lines, 1);
        assert!(preview.contains("-old line"), "preview: {preview}");
        assert!(preview.contains("+new line"), "preview: {preview}");
        assert!(preview.contains("+extra"), "preview: {preview}");
    }

    #[test]
    fn file_edit_empty_old_string_errors() {
        let base = unique_temp_dir("loong-file-edit-empty");
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
        let base = unique_temp_dir("loong-file-edit-escape");
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
        let base = unique_temp_dir("loong-glob-search");
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
        let base = unique_temp_dir("loong-content-search");
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
        let base = unique_temp_dir("loong-content-search-root");
        let root = base.join("root");
        let include = root.join("include");
        let exclude = root.join("exclude");
        fs::create_dir_all(&include).expect("create include");
        fs::create_dir_all(&exclude).expect("create exclude");
        fs::write(include.join("a.txt"), "needle here").expect("write include");
        fs::write(exclude.join("b.txt"), "needle here too").expect("write exclude");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
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

    #[test]
    fn content_search_does_not_mark_exact_limit_files_as_truncated() {
        let base = unique_temp_dir("loong-content-search-exact-limit");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("exact.txt"), "hello").expect("write exact-limit file");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "content.search".to_owned(),
            payload: json!({
                "query": "hello",
                "max_bytes_per_file": 5
            }),
        };
        let outcome = execute_content_search_tool_with_config(request, &config)
            .expect("content search succeeds");
        let matches = outcome.payload["matches"]
            .as_array()
            .expect("matches array");
        let first = matches.first().expect("first match");

        assert_eq!(first["truncated_file"], false);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn content_search_handles_unicode_case_insensitive_matches() {
        let base = unique_temp_dir("loong-content-search-unicode");
        let root = base.join("root");
        fs::create_dir_all(&root).expect("create root");
        fs::write(root.join("city.txt"), "Key value\n").expect("write city");

        let config = ToolRuntimeConfig {
            file_root: Some(root),
            ..ToolRuntimeConfig::default()
        };
        let request = ToolCoreRequest {
            tool_name: "content.search".to_owned(),
            payload: json!({
                "query": "key",
                "case_sensitive": false
            }),
        };
        let outcome = execute_content_search_tool_with_config(request, &config)
            .expect("content search succeeds");
        let matches = outcome.payload["matches"]
            .as_array()
            .expect("matches array");
        let first = matches.first().expect("first match");

        assert_eq!(first["path"], "city.txt");
        assert_eq!(first["match_text"], "Key");
        let _ = fs::remove_dir_all(base);
    }
}
