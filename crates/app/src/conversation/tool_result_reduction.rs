use std::borrow::Cow;

use serde_json::Value;

use super::tool_result_compaction::compact_tool_search_payload_summary_str;
use super::tool_result_line::ToolResultLine;

pub(crate) const FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS: usize = 384;
pub(crate) const SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS: usize = 384;
pub(crate) const SHELL_FOLLOWUP_STDIO_OMISSION_MARKER: &str = "\n[... omitted ...]\n";

pub(crate) fn reduce_followup_payload_for_model<'a>(label: &str, text: &'a str) -> Cow<'a, str> {
    if label != "tool_result" {
        return Cow::Borrowed(text);
    }

    reduce_tool_result_text_for_model(text)
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(text))
}

pub(crate) fn reduce_tool_result_text_for_model(text: &str) -> Option<String> {
    let mut changed = false;
    let reduced_lines = text
        .lines()
        .map(|line| {
            let reduced = reduce_tool_result_line_for_model(line);
            if reduced != line {
                changed = true;
            }
            reduced
        })
        .collect::<Vec<_>>();
    if !changed {
        return None;
    }
    let mut reduced = reduced_lines.join("\n");
    if text.ends_with('\n') {
        reduced.push('\n');
    }
    Some(reduced)
}

fn reduce_tool_result_line_for_model(line: &str) -> String {
    let Some(mut tool_result_line) = ToolResultLine::parse(line) else {
        return line.to_owned();
    };
    let canonical_tool_name = crate::tools::canonical_tool_name(tool_result_line.tool_name());
    let visible_tool_name = crate::tools::user_visible_tool_name(canonical_tool_name);
    let payload_truncated = tool_result_line.payload_truncated();
    let payload_summary = tool_result_line.payload_summary_str();

    let reduction = if payload_summary.is_empty() {
        None
    } else {
        match canonical_tool_name {
            "file.read" => {
                let Ok(payload_json) = serde_json::from_str::<Value>(payload_summary) else {
                    return line.to_owned();
                };
                reduce_file_read_payload_summary(&payload_json).map(|summary| (summary, true))
            }
            "shell.exec" => {
                let Ok(mut payload_json) = serde_json::from_str::<Value>(payload_summary) else {
                    return line.to_owned();
                };
                reduce_shell_payload_summary(&mut payload_json).map(|summary| (summary, true))
            }
            _ if !payload_truncated => compact_tool_search_payload_summary_str(payload_summary)
                .map(|summary| (summary, false)),
            _ => None,
        }
    };

    if reduction.is_none() && visible_tool_name == canonical_tool_name {
        return line.to_owned();
    }

    tool_result_line.set_tool_name(visible_tool_name);

    if let Some((reduced_summary, mark_truncated)) = reduction {
        if mark_truncated {
            tool_result_line.set_payload_truncated(true);
        }
        tool_result_line.replace_payload_summary_str(reduced_summary);
    }

    tool_result_line.render().unwrap_or_else(|| line.to_owned())
}

fn reduce_file_read_payload_summary(payload: &Value) -> Option<String> {
    let payload_object = payload.as_object()?;
    let (content_preview, content_chars, content_truncated) =
        summarize_file_read_content_preview(payload_object.get("content"));
    if !content_truncated {
        return None;
    }
    serde_json::to_string(&serde_json::json!({
        "path": payload_object.get("path").cloned().unwrap_or(Value::Null),
        "bytes": payload_object.get("bytes").cloned().unwrap_or(Value::Null),
        "truncated": payload_object.get("truncated").cloned().unwrap_or(Value::Null),
        "content_preview": content_preview,
        "content_chars": content_chars,
        "content_truncated": content_truncated,
    }))
    .ok()
}

fn reduce_shell_payload_summary(payload: &mut Value) -> Option<String> {
    let payload_object = payload.as_object_mut()?;
    let stdout_truncated = replace_shell_stdio_with_preview(payload_object, "stdout");
    let stderr_truncated = replace_shell_stdio_with_preview(payload_object, "stderr");
    if !stdout_truncated && !stderr_truncated {
        return None;
    }
    serde_json::to_string(payload).ok()
}

fn replace_shell_stdio_with_preview(
    payload_object: &mut serde_json::Map<String, Value>,
    field: &str,
) -> bool {
    let (preview, chars, truncated) = summarize_shell_output_preview(payload_object.get(field));
    if !truncated {
        return false;
    }
    payload_object.remove(field);
    payload_object.insert(format!("{field}_preview"), Value::String(preview));
    payload_object.insert(format!("{field}_chars"), serde_json::json!(chars));
    payload_object.insert(format!("{field}_truncated"), Value::Bool(true));
    true
}

fn summarize_file_read_content_preview(value: Option<&Value>) -> (String, usize, bool) {
    let text = value.and_then(Value::as_str).unwrap_or_default();
    let total_chars = text.chars().count();
    if total_chars <= FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS {
        return (text.to_owned(), total_chars, false);
    }
    (
        text.chars()
            .take(FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS)
            .collect(),
        total_chars,
        true,
    )
}

fn summarize_shell_output_preview(value: Option<&Value>) -> (String, usize, bool) {
    let text = value.and_then(Value::as_str).unwrap_or_default();
    let total_chars = text.chars().count();
    if total_chars <= SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS {
        return (text.to_owned(), total_chars, false);
    }
    let marker_chars = SHELL_FOLLOWUP_STDIO_OMISSION_MARKER.chars().count();
    let Some(available_chars) = SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS.checked_sub(marker_chars) else {
        return (
            text.chars()
                .take(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS)
                .collect(),
            total_chars,
            true,
        );
    };
    if available_chars < 2 {
        return (
            text.chars()
                .take(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS)
                .collect(),
            total_chars,
            true,
        );
    }

    let tail_chars = available_chars / 2;
    let head_chars = available_chars - tail_chars;
    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text.chars().skip(total_chars - tail_chars).collect();

    (
        format!("{head}{SHELL_FOLLOWUP_STDIO_OMISSION_MARKER}{tail}"),
        total_chars,
        true,
    )
}
