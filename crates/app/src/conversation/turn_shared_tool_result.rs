use std::borrow::Cow;

use serde_json::Value;

use super::super::tool_result_compaction::compact_tool_search_payload_summary_str;
use super::{
    FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS, SHELL_FOLLOWUP_STDIO_OMISSION_MARKER,
    SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS, ToolResultLine, ToolResultPayloadSemantics,
};

pub fn tool_result_contains_truncation_signal(tool_result_text: &str) -> bool {
    let normalized = tool_result_text.to_ascii_lowercase();
    normalized.contains("...(truncated ")
        || normalized.contains("... (truncated ")
        || normalized.contains("[tool_result_truncated]")
        || tool_result_text
            .lines()
            .any(line_contains_structured_truncation_signal)
}

pub(super) fn proactive_followup_continuation_context(
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> Option<String> {
    let primary_context = tool_result_text.and_then(parse_tool_result_followup_context);
    let fallback_context = rendered_tool_result_text.and_then(parse_tool_result_followup_context);
    let tool_result_context = primary_context.or(fallback_context)?;
    let continuation = parse_tool_result_continuation(&tool_result_context.payload_json)?;
    Some(render_tool_result_continuation_guidance(&continuation))
}

pub(super) fn followup_prompt_needs_truncation_hint(
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> bool {
    tool_result_text
        .map(tool_result_contains_truncation_signal)
        .unwrap_or(false)
        || rendered_tool_result_text
            .map(tool_result_contains_truncation_signal)
            .unwrap_or(false)
}

pub(super) fn followup_prompt_uses_discovery_guidance(
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> bool {
    tool_result_text
        .map(tool_result_contains_discovery_payload)
        .unwrap_or(false)
        || rendered_tool_result_text
            .map(tool_result_contains_discovery_payload)
            .unwrap_or(false)
}

pub fn reduce_followup_payload_for_model<'a>(label: &str, text: &'a str) -> Cow<'a, str> {
    if label != "tool_result" {
        return Cow::Borrowed(text);
    }

    reduce_tool_result_text_for_model(text)
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(text))
}

fn line_contains_structured_truncation_signal(line: &str) -> bool {
    let Some(envelope) = parse_tool_result_envelope(line) else {
        return false;
    };
    envelope
        .get("payload_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn parse_tool_result_envelope(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(tool_result_line) = ToolResultLine::parse(trimmed) {
        return serde_json::to_value(tool_result_line.envelope()).ok();
    }
    let candidate = if trimmed.starts_with('[') {
        trimmed
            .split_once(' ')
            .map(|(_, payload)| payload.trim())
            .unwrap_or("")
    } else {
        trimmed
    };
    if !(candidate.starts_with('{') || candidate.starts_with('[')) {
        return None;
    }
    serde_json::from_str::<Value>(candidate).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolResultFollowupContext {
    pub(super) payload_json: Value,
}

pub(super) fn parse_tool_result_followup_context(
    tool_result_text: &str,
) -> Option<ToolResultFollowupContext> {
    tool_result_text.lines().find_map(|line| {
        let trimmed_line = line.trim();
        let tool_result_line = ToolResultLine::parse(trimmed_line)?;
        let payload_json = tool_result_line.payload_summary_json()?;
        Some(ToolResultFollowupContext { payload_json })
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolResultContinuation {
    pub(super) state: String,
    pub(super) is_terminal: bool,
    pub(super) recommended_tool: Option<String>,
    pub(super) recommended_payload: Option<Value>,
    pub(super) note: Option<String>,
}

pub(super) fn parse_tool_result_continuation(
    payload_json: &Value,
) -> Option<ToolResultContinuation> {
    let continuation_value = payload_json.get("continuation")?;
    let continuation_object = continuation_value.as_object()?;
    let state = continuation_object
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let is_terminal = continuation_object
        .get("is_terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let recommended_tool = continuation_object
        .get("recommended_tool")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let recommended_payload = continuation_object.get("recommended_payload").cloned();
    let note = continuation_object
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some(ToolResultContinuation {
        state,
        is_terminal,
        recommended_tool,
        recommended_payload,
        note,
    })
}

fn render_tool_result_continuation_guidance(continuation: &ToolResultContinuation) -> String {
    let mut lines = vec!["Continuation guidance:".to_owned()];
    let state_line = if continuation.is_terminal {
        format!(
            "The tool reported terminal state `{}`. Finish the request only if the user-facing work is actually complete.",
            continuation.state
        )
    } else {
        format!(
            "The tool reported intermediate state `{}`. Do not present this as final completion.",
            continuation.state
        )
    };
    lines.push(state_line);

    if let Some(note) = continuation.note.as_deref() {
        lines.push(note.to_owned());
    }

    if let Some(recommended_tool) = continuation.recommended_tool.as_deref() {
        let mut recommendation = format!(
            "If the original request still depends on this work, continue with `{recommended_tool}`"
        );
        if let Some(recommended_payload) = continuation.recommended_payload.as_ref() {
            let payload_text =
                serde_json::to_string(recommended_payload).unwrap_or_else(|_| "{}".to_owned());
            recommendation.push_str(" using payload:");
            recommendation.push('\n');
            recommendation.push_str(payload_text.as_str());
            recommendation.push('\n');
            recommendation.push_str("before answering.");
        } else {
            recommendation.push_str(" before answering.");
        }
        lines.push(recommendation);
    } else if !continuation.is_terminal {
        lines.push(
            "Keep advancing if you can resolve the gate from available tools; otherwise report the exact blocker instead of a vague progress update."
                .to_owned(),
        );
    }

    lines.join("\n")
}

fn reduce_tool_result_text_for_model(text: &str) -> Option<String> {
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

fn tool_result_contains_discovery_payload(tool_result_text: &str) -> bool {
    tool_result_text
        .lines()
        .filter_map(parse_tool_result_envelope)
        .any(|envelope| envelope_uses_discovery_semantics(&envelope))
}

fn envelope_uses_discovery_semantics(envelope: &Value) -> bool {
    let uses_explicit_semantics =
        envelope_has_payload_semantics(envelope, ToolResultPayloadSemantics::DiscoveryResult);
    if uses_explicit_semantics {
        return true;
    }
    envelope_contains_discovery_payload(envelope)
}

pub(super) fn envelope_uses_external_skill_context(envelope: &Value) -> bool {
    let uses_explicit_semantics =
        envelope_has_payload_semantics(envelope, ToolResultPayloadSemantics::ExternalSkillContext);
    if uses_explicit_semantics {
        return true;
    }

    envelope_uses_legacy_external_skill_tool(envelope)
}

fn envelope_uses_legacy_external_skill_tool(envelope: &Value) -> bool {
    let tool_name = envelope.get("tool").and_then(Value::as_str);
    tool_name == Some("external_skills.invoke")
}

fn envelope_contains_discovery_payload(envelope: &Value) -> bool {
    let Some(payload_summary) = envelope.get("payload_summary").and_then(Value::as_str) else {
        return false;
    };
    let Ok(payload_json) = serde_json::from_str::<Value>(payload_summary) else {
        return false;
    };
    payload_summary_looks_like_discovery_result(&payload_json)
}

fn payload_summary_looks_like_discovery_result(payload: &Value) -> bool {
    let Some(payload_object) = payload.as_object() else {
        return false;
    };
    let Some(results) = payload_object.get("results").and_then(Value::as_array) else {
        return false;
    };

    if results.is_empty() {
        return payload_object.contains_key("query");
    }

    results.iter().any(|result| {
        let Some(result_object) = result.as_object() else {
            return false;
        };
        result_object
            .get("tool_id")
            .and_then(Value::as_str)
            .is_some()
            && result_object.get("lease").and_then(Value::as_str).is_some()
    })
}

fn envelope_payload_semantics(envelope: &Value) -> Option<ToolResultPayloadSemantics> {
    let payload_semantics_value = envelope.get("payload_semantics")?;
    serde_json::from_value(payload_semantics_value.clone()).ok()
}

fn envelope_has_payload_semantics(
    envelope: &Value,
    expected_semantics: ToolResultPayloadSemantics,
) -> bool {
    envelope_payload_semantics(envelope) == Some(expected_semantics)
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
