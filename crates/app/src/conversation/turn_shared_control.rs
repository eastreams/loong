use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::ToolDrivenFollowupPayload;

const TOOL_FOLLOWUP_REPAIR_PROMPT: &str = "Repair notice:\nThe previous reply described a next step without issuing the required tool call. Do not describe the plan again.";
const TOOL_FOLLOWUP_RETRYABLE_FAILURE_PROMPT: &str = "The previous tool failure was retryable. Default to [followup_state:continue] and retry or repair the tool call unless the task is already complete or genuinely blocked.";
const FOLLOWUP_STATE_MARKER_PREFIX: &str = "[followup_state:";
const MISSING_TOOL_CALL_REASON_PREFIX: &str = "missing_tool_call_followup:";
pub(crate) const MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS: usize = 240;
const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

pub(crate) fn sanitize_reply_text(text: &str) -> String {
    parse_tool_driven_continuation_reply(text).reply
}

pub fn missing_tool_call_followup_payload(reply_text: &str) -> Option<ToolDrivenFollowupPayload> {
    let sanitized_reply = sanitize_reply_text(reply_text);
    let detection_kind = detect_missing_tool_call_kind(sanitized_reply.as_str())?;
    let excerpt = truncated_missing_tool_call_excerpt(sanitized_reply.as_str());
    let reason = match detection_kind {
        MissingToolCallKind::EmptyFollowup => format!(
            "{MISSING_TOOL_CALL_REASON_PREFIX} previous assistant reply ended the tool-followup round without any content or tool call. If more tool work is needed, emit the exact next tool call now instead of returning an empty follow-up."
        ),
        MissingToolCallKind::PseudoToolCommand => format!(
            "{MISSING_TOOL_CALL_REASON_PREFIX} previous assistant reply emitted pseudo-tool text instead of a real tool call. If another tool is required, emit the exact next tool call now instead of formatting it as plain text.\nReply excerpt:\n{excerpt}"
        ),
        MissingToolCallKind::PseudoToolMarkup => format!(
            "{MISSING_TOOL_CALL_REASON_PREFIX} previous assistant reply emitted malformed tool-call markup instead of a real tool call. If another tool is required, emit the exact next tool call now instead of leaking tool wrapper text.\nReply excerpt:\n{excerpt}"
        ),
    };

    Some(ToolDrivenFollowupPayload::ToolFailure {
        reason,
        retryable: true,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingToolCallKind {
    EmptyFollowup,
    PseudoToolCommand,
    PseudoToolMarkup,
}

fn detect_missing_tool_call_kind(reply_text: &str) -> Option<MissingToolCallKind> {
    let normalized_reply = reply_text.trim();
    if normalized_reply.is_empty() {
        return Some(MissingToolCallKind::EmptyFollowup);
    }

    if contains_pseudo_tool_markup(normalized_reply) {
        return Some(MissingToolCallKind::PseudoToolMarkup);
    }

    normalized_reply
        .lines()
        .map(str::trim)
        .any(line_looks_like_pseudo_tool_command)
        .then_some(MissingToolCallKind::PseudoToolCommand)
}

fn contains_pseudo_tool_markup(reply_text: &str) -> bool {
    let normalized_reply = reply_text.trim();

    if normalized_reply.starts_with("[tool_request]")
        || normalized_reply.starts_with("[tool_failure]")
    {
        return true;
    }

    let contains_tool_marker =
        normalized_reply.contains("[tool_request]") || normalized_reply.contains("[tool_failure]");
    let contains_json_tool_shape = normalized_reply.contains('{')
        && normalized_reply.contains('}')
        && (normalized_reply.contains("\"name\"")
            || normalized_reply.contains("\"tool\"")
            || normalized_reply.contains("\"tool_name\""))
        && (normalized_reply.contains("\"arguments\"") || normalized_reply.contains("\"request\""));

    contains_tool_marker || contains_json_tool_shape
}

fn line_looks_like_pseudo_tool_command(line: &str) -> bool {
    if let Some(trimmed_line) = line.strip_prefix('/') {
        let Some((surface, remainder)) = trimmed_line.split_once(':') else {
            return false;
        };
        let has_surface = !surface.trim().is_empty();
        let has_remainder = !remainder.trim().is_empty();
        let surface_is_tool_like = surface
            .chars()
            .all(|character| character.is_ascii_lowercase() || ".-_".contains(character));

        return has_surface && has_remainder && surface_is_tool_like;
    }

    let Some(trimmed_line) = line.strip_prefix("to=") else {
        return false;
    };
    let mut parts = trimmed_line.split_whitespace();
    let Some(surface) = parts.next() else {
        return false;
    };
    let remainder = parts.collect::<Vec<_>>().join(" ");
    let surface_is_tool_like = surface
        .chars()
        .all(|character| character.is_ascii_lowercase() || ".-_".contains(character));
    let has_remainder = remainder.contains("content=")
        || remainder.contains("json")
        || remainder.contains('{')
        || remainder.contains(':');

    !surface.is_empty() && surface_is_tool_like && has_remainder
}

fn truncated_missing_tool_call_excerpt(reply_text: &str) -> String {
    let total_chars = reply_text.chars().count();
    if total_chars <= MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS {
        return reply_text.to_owned();
    }

    let truncated_reply = reply_text
        .chars()
        .take(MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS)
        .collect::<String>();

    format!(
        "{truncated_reply}\n[reply_excerpt_truncated] omitted_chars={}",
        total_chars - MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS
    )
}

pub fn next_conversation_turn_id() -> String {
    static NEXT_CONVERSATION_TURN_SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = NEXT_CONVERSATION_TURN_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("turn-{nanos:x}-{seq:x}")
}

pub fn tool_loop_circuit_breaker_reply(
    prospective_total: usize,
    max_total_tool_calls: usize,
) -> Option<String> {
    (prospective_total > max_total_tool_calls).then(|| {
        format!(
            "tool_loop_circuit_breaker: would exceed {}/{} tool calls this turn. Do you want to continue? Reply to resume.",
            prospective_total, max_total_tool_calls
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDrivenContinuationState {
    Continue,
    Done,
    Blocked,
}

impl ToolDrivenContinuationState {
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Continue => "[followup_state:continue]",
            Self::Done => "[followup_state:done]",
            Self::Blocked => "[followup_state:blocked]",
        }
    }

    fn parse_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "continue" => Some(Self::Continue),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolDrivenFollowupContractMode {
    RetryableFailure,
    RepairRetryableFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolDrivenContinuationReply {
    pub state: Option<ToolDrivenContinuationState>,
    pub reply: String,
}

pub fn parse_tool_driven_continuation_reply(text: &str) -> ParsedToolDrivenContinuationReply {
    let stripped_text = strip_think_tags(text);
    let trimmed_text = stripped_text.trim();
    let Some(rest) = trimmed_text.strip_prefix(FOLLOWUP_STATE_MARKER_PREFIX) else {
        return ParsedToolDrivenContinuationReply {
            state: None,
            reply: trimmed_text.to_owned(),
        };
    };
    let Some((state_token, remainder)) = rest.split_once(']') else {
        return ParsedToolDrivenContinuationReply {
            state: None,
            reply: trimmed_text.to_owned(),
        };
    };
    let Some(state) = ToolDrivenContinuationState::parse_token(state_token) else {
        return ParsedToolDrivenContinuationReply {
            state: None,
            reply: trimmed_text.to_owned(),
        };
    };

    ParsedToolDrivenContinuationReply {
        state: Some(state),
        reply: remainder.trim().to_owned(),
    }
}

pub(crate) fn render_tool_followup_continuation_contract(
    mode: ToolDrivenFollowupContractMode,
) -> String {
    let mut sections = Vec::new();
    if matches!(mode, ToolDrivenFollowupContractMode::RepairRetryableFailure) {
        sections.push(TOOL_FOLLOWUP_REPAIR_PROMPT.to_owned());
    }
    if matches!(
        mode,
        ToolDrivenFollowupContractMode::RetryableFailure
            | ToolDrivenFollowupContractMode::RepairRetryableFailure
    ) {
        sections.push(TOOL_FOLLOWUP_RETRYABLE_FAILURE_PROMPT.to_owned());
    }
    sections.push(format!(
        "Structured continuation contract:\n- Start your reply with exactly one marker: {}, {}, or {}.\n- If you choose continue, emit the next tool call now. Do not only describe a plan.\n- If you choose done, give the completed final answer now.\n- If you choose blocked, explain the blocker briefly and do not claim the task is running or complete.",
        ToolDrivenContinuationState::Continue.marker(),
        ToolDrivenContinuationState::Done.marker(),
        ToolDrivenContinuationState::Blocked.marker(),
    ));
    sections.join("\n\n")
}

pub(crate) fn strip_think_tags(text: &str) -> String {
    let mut cleaned_text = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut think_depth = 0usize;

    while cursor < text.len() {
        let remaining_text = &text[cursor..];
        let open_tag_length = think_tag_prefix_len(remaining_text, THINK_OPEN_TAG);

        if let Some(tag_length) = open_tag_length {
            think_depth = think_depth.saturating_add(1);
            cursor += tag_length;
            continue;
        }

        let close_tag_length = think_tag_prefix_len(remaining_text, THINK_CLOSE_TAG);

        if let Some(tag_length) = close_tag_length {
            think_depth = think_depth.saturating_sub(1);
            cursor += tag_length;
            continue;
        }

        let mut remaining_chars = remaining_text.chars();
        let Some(current_char) = remaining_chars.next() else {
            break;
        };
        let current_char_length = current_char.len_utf8();

        if think_depth == 0 {
            cleaned_text.push(current_char);
        }

        cursor += current_char_length;
    }

    cleaned_text
}

fn think_tag_prefix_len(input: &str, tag: &str) -> Option<usize> {
    let tag_length = tag.len();
    let input_prefix = input.get(..tag_length)?;
    let matches_tag = input_prefix.eq_ignore_ascii_case(tag);

    if !matches_tag {
        return None;
    }

    Some(tag_length)
}
