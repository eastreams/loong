use std::borrow::Cow;

use serde_json::Value;

use super::super::tool_result_reduction::reduce_tool_result_text_for_model;
use super::{ToolResultLine, ToolResultPayloadSemantics};

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
    if let Some(continuation) = parse_tool_result_continuation(&tool_result_context.payload_json) {
        return Some(render_tool_result_continuation_guidance(&continuation));
    }
    render_tool_result_partial_evidence_guidance(&tool_result_context.payload_json)
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
        return Some(serde_json::json!({
            "tool": tool_result_line.tool_name(),
            "payload_summary": tool_result_line.payload_summary_str(),
            "payload_truncated": tool_result_line.payload_truncated(),
        }));
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
pub(crate) struct ToolResultFollowupContext {
    pub(crate) payload_json: Value,
}

pub(crate) fn parse_tool_result_followup_context(
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
pub(crate) struct ToolResultContinuation {
    pub(crate) state: String,
    pub(crate) is_terminal: bool,
    pub(crate) recommended_tool: Option<String>,
    pub(crate) recommended_payload: Option<Value>,
    pub(crate) note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolResultContinuationKind {
    PathListing,
    InsufficientPageEvidence,
    Other,
}

impl ToolResultContinuation {
    pub(crate) fn kind(&self) -> ToolResultContinuationKind {
        match (self.state.as_str(), self.recommended_tool.as_deref()) {
            ("path_listing", _) => ToolResultContinuationKind::PathListing,
            ("insufficient_page_evidence", Some("web" | "browse")) => {
                ToolResultContinuationKind::InsufficientPageEvidence
            }
            _ => ToolResultContinuationKind::Other,
        }
    }

    pub(crate) fn reply_requests_more_evidence(&self, reply: &str) -> bool {
        let normalized = reply.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        let mentions_more_work = normalized.contains("still need")
            || normalized.contains("need one more")
            || normalized.contains("do not yet have usable")
            || normalized.contains("could not reliably retrieve")
            || normalized.contains("not enough evidence")
            || normalized.contains("gather enough evidence")
            || normalized.contains("finish the summary")
            || normalized.contains("ground the summary");
        let requests_permission_like_followup = normalized.contains("please allow")
            || normalized.contains("allow another")
            || normalized.contains("another wait")
            || normalized.contains("another read")
            || normalized.contains("another fetch")
            || normalized.contains("another inspect")
            || normalized.contains("inspection step")
            || normalized.contains("without another read")
            || normalized.contains("without a successful fetch")
            || normalized.contains("should not claim more specific");

        let matches_structured_continuation_context = match self.kind() {
            ToolResultContinuationKind::PathListing => {
                normalized.contains("ground the summary")
                    || normalized.contains("top-level docs")
                    || normalized.contains("actual docs")
            }
            ToolResultContinuationKind::InsufficientPageEvidence => {
                normalized.contains("narrower browser extract")
                    || normalized.contains("narrower fetch")
                    || normalized.contains("shell-heavy navigation")
            }
            ToolResultContinuationKind::Other => false,
        };

        mentions_more_work
            && (requests_permission_like_followup || matches_structured_continuation_context)
    }
}

pub(crate) fn parse_tool_result_continuation(
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

fn render_tool_result_partial_evidence_guidance(payload_json: &Value) -> Option<String> {
    let matches = payload_json.get("matches")?.as_array()?;
    if matches.is_empty() {
        return None;
    }

    Some(
        "Continuation guidance:\nThe last read result only listed matching paths. If the user still needs file contents or a grounded repository summary, continue with another direct `read` call for the highest-value files instead of stopping at the listing."
            .to_owned(),
    )
}

pub(super) fn envelope_uses_skill_context(envelope: &Value) -> bool {
    envelope_has_payload_semantics(envelope, ToolResultPayloadSemantics::SkillContext)
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
