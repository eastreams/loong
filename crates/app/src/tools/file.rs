use std::{fs, path::Path};

use super::file_path::resolve_safe_file_path_with_config;
#[cfg(feature = "tool-file")]
use super::runtime_events::{
    ToolFileChangeKind, ToolFileChangePreview, ToolRuntimeEvent, current_tool_runtime_event_sink,
};
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;
#[cfg(feature = "tool-file")]
use serde_json::json;

#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_LINES: usize = 8;
#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_CHARS: usize = 1_200;
#[cfg(feature = "tool-file")]
const FILE_CHANGE_PREVIEW_MAX_COMPARISON_CELLS: usize = 200_000;

#[cfg(feature = "tool-file")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactTextEditBlock {
    old_text: String,
    new_text: String,
}

#[cfg(feature = "tool-file")]
enum FileEditRequest {
    ExactBlocks(Vec<ExactTextEditBlock>),
}

#[cfg(feature = "tool-file")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedExactTextEditBlock<'a> {
    start: usize,
    end: usize,
    block: &'a ExactTextEditBlock,
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
    tool_name: &str,
) -> Result<Option<Vec<ExactTextEditBlock>>, String> {
    let Some(raw_blocks) = payload.get("edits") else {
        return Ok(None);
    };
    let blocks = raw_blocks
        .as_array()
        .ok_or_else(|| format!("{tool_name} payload.edits must be an array"))?;
    if blocks.is_empty() {
        return Err(format!(
            "{tool_name} payload.edits must contain at least one edit block"
        ));
    }

    let mut parsed_blocks = Vec::with_capacity(blocks.len());
    for (index, raw_block) in blocks.iter().enumerate() {
        let block = raw_block
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload.edits[{index}] must be an object"))?;
        let old_text = exact_edit_block_field(block, "old_text", "oldText").ok_or_else(|| {
            format!("{tool_name} payload.edits[{index}].old_text must be a string")
        })?;
        if old_text.is_empty() {
            return Err(format!(
                "edit_failed: edits[{index}].old_text must not be empty"
            ));
        }
        let new_text = exact_edit_block_field(block, "new_text", "newText").ok_or_else(|| {
            format!("{tool_name} payload.edits[{index}].new_text must be a string")
        })?;
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
    tool_name: &str,
) -> Result<FileEditRequest, String> {
    let parsed_blocks = parse_exact_edit_blocks(payload, tool_name)?;
    if let Some(blocks) = parsed_blocks {
        return Ok(FileEditRequest::ExactBlocks(blocks));
    }

    Err(format!("{tool_name} requires payload.edits"))
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
        let tool_name = super::legacy_display_tool_name(request.tool_name.as_str());
        let payload = request
            .payload
            .as_object()
            .ok_or_else(|| format!("{tool_name} payload must be an object"))?;

        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{tool_name} requires payload.path (string)"))?;
        let edit_request = parse_file_edit_request(payload, tool_name.as_str())?;

        let resolved = resolve_safe_file_path_with_config(path, config)?;
        let content = fs::read_to_string(&resolved)
            .map_err(|e| format!("failed to read {}: {e}", resolved.display()))?;

        let (updated, replacements_made) = match &edit_request {
            FileEditRequest::ExactBlocks(blocks) => {
                apply_exact_edit_blocks(content.as_str(), blocks)
            }
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
        };
        let mut response_payload = json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "path": resolved.display().to_string(),
            "replacements_made": replacements_made,
            "bytes_written": updated.len(),
        });
        if let Some(response_object) = response_payload.as_object_mut() {
            if let Some(edit_blocks_applied) = edit_blocks_applied {
                response_object
                    .insert("edit_blocks_applied".to_owned(), json!(edit_blocks_applied));
            }
            response_object.insert(
                "continuation".to_owned(),
                json!({
                    "state": "verify_file_change",
                    "is_terminal": false,
                    "recommended_tool": "read",
                    "recommended_payload": {
                        "path": resolved.display().to_string()
                    },
                    "note": "If the user still depends on the updated file contents, verify the file before finalizing."
                }),
            );
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

#[cfg(all(test, feature = "tool-file"))]
mod tests;
