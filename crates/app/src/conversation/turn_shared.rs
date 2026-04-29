use super::super::config::LoongConfig;
use super::ProviderErrorMode;
use super::persistence::format_provider_error_reply;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::tool_input_contract::{
    render_tool_input_repair_guidance, render_tool_input_repair_guidance_from_reason,
    repair_guidance_visible_tool_name,
};
use super::tool_result_line::ToolResultLine;
#[cfg(test)]
use super::tool_result_reduction::SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS;
#[cfg(test)]
use super::turn_engine::ToolResultEnvelope;
use super::turn_engine::{
    ApprovalRequirement, ApprovalRequirementKind, ProviderTurn, ToolBatchExecutionIntentStatus,
    ToolBatchExecutionTrace, ToolIntent, ToolResultPayloadSemantics, TurnFailure, TurnResult,
};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

use crate::CliResult;

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to continue satisfying the original user request. Prefer the next bounded tool call or completion step over narrating intermediate status. Only stop to answer in natural language when the request is actually complete, blocked on a real approval or input gate, or the available evidence is already sufficient. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
pub const DISCOVERY_RESULT_FOLLOWUP_PROMPT: &str = "The tool result above is a discovery result, not the final evidence. Choose the best matching discovered tool, reuse its lease when invoking it, continue with the next tool call needed to satisfy the original user request, and only answer directly if the discovery results already contain the final user-facing information.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";
pub const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "An external skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";
pub const DISCOVERY_RECOVERY_FOLLOWUP_PROMPT: &str = "The previous tool call could not be executed as requested. If you still need a hidden or discoverable capability, call tool.search with a short natural-language description of the missing capability. If tool.search returns a grouped hidden surface such as `skills`, `agent`, or `channel`, do not call that surface name directly; reuse its fresh lease through tool.invoke and place the requested operation inside payload.arguments. Otherwise, provide the best possible answer with the currently available evidence.";
pub const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";
const TOOL_FOLLOWUP_REPAIR_PROMPT: &str = "Repair notice:\nThe previous reply described a next step without issuing the required tool call. Do not describe the plan again.";
const TOOL_FOLLOWUP_RETRYABLE_FAILURE_PROMPT: &str = "The previous tool failure was retryable. Default to [followup_state:continue] and retry or repair the tool call unless the task is already complete or genuinely blocked.";
const FOLLOWUP_STATE_MARKER_PREFIX: &str = "[followup_state:";
const MISSING_TOOL_CALL_REASON_PREFIX: &str = "missing_tool_call_followup:";
const MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS: usize = 240;

const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";

/// Strips <think>...</think> tags from model response text to prevent
/// internal reasoning chains from leaking to user-facing output.
/// This handles both standard think tags and case-insensitive variants.
fn strip_think_tags(text: &str) -> String {
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

fn sanitize_reply_text(text: &str) -> String {
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
}

fn detect_missing_tool_call_kind(reply_text: &str) -> Option<MissingToolCallKind> {
    let normalized_reply = reply_text.trim();
    if normalized_reply.is_empty() {
        return Some(MissingToolCallKind::EmptyFollowup);
    }

    normalized_reply
        .lines()
        .map(str::trim)
        .any(line_looks_like_pseudo_tool_command)
        .then_some(MissingToolCallKind::PseudoToolCommand)
}

fn line_looks_like_pseudo_tool_command(line: &str) -> bool {
    let Some(trimmed_line) = line.strip_prefix('/') else {
        return false;
    };
    let Some((surface, remainder)) = trimmed_line.split_once(':') else {
        return false;
    };
    let has_surface = !surface.trim().is_empty();
    let has_remainder = !remainder.trim().is_empty();
    let surface_is_tool_like = surface
        .chars()
        .all(|character| character.is_ascii_lowercase() || ".-_".contains(character));

    has_surface && has_remainder && surface_is_tool_like
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDrivenFollowupPayload {
    ToolResult { text: String },
    ToolFailure { reason: String, retryable: bool },
    DiscoveryRecovery { reason: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDrivenFollowupKind {
    ToolResult,
    ToolFailure,
    DiscoveryRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDrivenFollowupLabel {
    ToolResult,
    ToolFailure,
    DiscoveryRecovery,
}

impl ToolDrivenFollowupLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolResult => "tool_result",
            Self::ToolFailure => "tool_failure",
            Self::DiscoveryRecovery => "tool_recovery",
        }
    }

    #[cfg(test)]
    pub fn from_marker(marker: &str) -> Option<Self> {
        match marker {
            "tool_result" => Some(Self::ToolResult),
            "tool_failure" => Some(Self::ToolFailure),
            "tool_recovery" => Some(Self::DiscoveryRecovery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FollowupPayloadBudget {
    per_round_max_chars: usize,
    remaining_total_chars: usize,
}

impl FollowupPayloadBudget {
    pub(crate) fn new(per_round_max_chars: usize, total_max_chars: usize) -> Self {
        Self {
            per_round_max_chars: per_round_max_chars.max(1),
            remaining_total_chars: total_max_chars,
        }
    }

    pub(crate) fn truncate_payload(
        &mut self,
        label: ToolDrivenFollowupLabel,
        text: &str,
    ) -> String {
        let label_text = label.as_str();
        self.truncate_payload_text_label(label_text, text)
    }

    pub(crate) fn truncate_payload_text_label(&mut self, label_text: &str, text: &str) -> String {
        let per_round_allowed = self
            .per_round_max_chars
            .min(self.remaining_total_chars.max(1));
        if self.remaining_total_chars == 0 {
            let removed = text.trim().chars().count();
            return format!(
                "[{label_text}_truncated] removed_chars={removed} budget_exhausted=true"
            );
        }

        let bounded = truncate_followup_tool_payload(label_text, text, per_round_allowed);
        let normalized = text.trim();
        let total_chars = normalized.chars().count();
        let consumed_chars = if total_chars <= per_round_allowed {
            total_chars
        } else if per_round_allowed > 80 {
            per_round_allowed - 80
        } else {
            per_round_allowed
        };
        self.remaining_total_chars = self.remaining_total_chars.saturating_sub(consumed_chars);
        bounded
    }
}

fn truncate_followup_tool_payload(label: &str, text: &str, max_chars: usize) -> String {
    let normalized = text.trim();
    let total_chars = normalized.chars().count();
    if total_chars <= max_chars {
        return normalized.to_owned();
    }

    let reserved_chars = 80usize;
    let keep_chars = max_chars.saturating_sub(reserved_chars).max(1);
    let truncated = normalized.chars().take(keep_chars).collect::<String>();
    let removed = total_chars.saturating_sub(keep_chars);
    format!("{truncated}\n[{label}_truncated] removed_chars={removed}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDrivenFollowupTextRef<'a> {
    label: ToolDrivenFollowupLabel,
    text: &'a str,
}

impl<'a> ToolDrivenFollowupTextRef<'a> {
    pub fn new(label: ToolDrivenFollowupLabel, text: &'a str) -> Self {
        Self { label, text }
    }

    pub fn label(self) -> ToolDrivenFollowupLabel {
        self.label
    }

    pub fn text(self) -> &'a str {
        self.text
    }

    pub fn render_assistant_content(self) -> String {
        let marker = self.label.as_str();
        format!("[{marker}]\n{}", self.text)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDrivenFollowupMessageOwned {
    label: ToolDrivenFollowupLabel,
    body: String,
}

#[cfg(test)]
impl ToolDrivenFollowupMessageOwned {
    pub fn parse_assistant_content(content: &str) -> Option<Self> {
        let (marker_line, body) = content.split_once('\n')?;
        let marker = marker_line.trim().strip_prefix('[')?.strip_suffix(']')?;
        let label = ToolDrivenFollowupLabel::from_marker(marker)?;
        Some(Self {
            label,
            body: body.to_owned(),
        })
    }

    pub fn label(&self) -> ToolDrivenFollowupLabel {
        self.label
    }

    pub fn body(&self) -> &str {
        self.body.as_str()
    }
}

impl ToolDrivenFollowupPayload {
    pub fn kind(&self) -> ToolDrivenFollowupKind {
        match self {
            Self::ToolResult { .. } => ToolDrivenFollowupKind::ToolResult,
            Self::ToolFailure { .. } => ToolDrivenFollowupKind::ToolFailure,
            Self::DiscoveryRecovery { .. } => ToolDrivenFollowupKind::DiscoveryRecovery,
        }
    }

    pub fn label(&self) -> ToolDrivenFollowupLabel {
        match self {
            Self::ToolResult { .. } => ToolDrivenFollowupLabel::ToolResult,
            Self::ToolFailure { .. } => ToolDrivenFollowupLabel::ToolFailure,
            Self::DiscoveryRecovery { .. } => ToolDrivenFollowupLabel::DiscoveryRecovery,
        }
    }

    pub fn message_context(&self) -> ToolDrivenFollowupTextRef<'_> {
        let label = self.label();
        match self {
            Self::ToolResult { text } => ToolDrivenFollowupTextRef::new(label, text.as_str()),
            Self::ToolFailure { reason, .. } => {
                ToolDrivenFollowupTextRef::new(label, reason.as_str())
            }
            Self::DiscoveryRecovery { reason } => {
                ToolDrivenFollowupTextRef::new(label, reason.as_str())
            }
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn retryable_failure(&self) -> bool {
        matches!(
            self,
            Self::ToolFailure {
                retryable: true,
                ..
            }
        )
    }

    pub fn requests_runtime_followup_chain(&self) -> bool {
        match self {
            Self::DiscoveryRecovery { .. } => true,
            Self::ToolResult { text } => {
                let tool_result_context = parse_tool_result_followup_context(text.as_str());
                let continuation = tool_result_context
                    .as_ref()
                    .and_then(|context| parse_tool_result_continuation(&context.payload_json));
                continuation.is_some_and(|continuation| !continuation.is_terminal)
            }
            Self::ToolFailure { .. } => false,
        }
    }
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

pub fn turn_failure_supports_discovery_recovery(failure: &TurnFailure) -> bool {
    failure.supports_discovery_recovery
}

pub fn tool_driven_followup_payload(
    had_tool_intents: bool,
    turn_result: &TurnResult,
) -> Option<ToolDrivenFollowupPayload> {
    if !had_tool_intents {
        return None;
    }

    match turn_result {
        TurnResult::FinalText(text)
        | TurnResult::StreamingText(text)
        | TurnResult::StreamingDone(text) => {
            let sanitized_text = sanitize_reply_text(text);
            Some(ToolDrivenFollowupPayload::ToolResult {
                text: sanitized_text,
            })
        }
        TurnResult::NeedsApproval(_) => None,
        TurnResult::ToolDenied(failure) if turn_failure_supports_discovery_recovery(failure) => {
            Some(ToolDrivenFollowupPayload::DiscoveryRecovery {
                reason: failure.reason.clone(),
            })
        }
        TurnResult::ToolDenied(failure) | TurnResult::ToolError(failure) => {
            Some(ToolDrivenFollowupPayload::ToolFailure {
                reason: failure.reason.clone(),
                retryable: failure.retryable,
            })
        }
        TurnResult::ProviderError(_) => None,
    }
}

pub(crate) fn summarize_tool_followup_request(intents: &[ToolIntent]) -> Option<String> {
    match intents {
        [] => None,
        [intent] => summarize_single_tool_followup_request(intent),
        intents => serde_json::to_string(
            &intents
                .iter()
                .map(tool_followup_request_entry)
                .collect::<Vec<_>>(),
        )
        .ok(),
    }
}

pub(crate) fn summarize_single_tool_followup_request(intent: &ToolIntent) -> Option<String> {
    let entry = tool_followup_request_entry(intent);
    serde_json::to_string(&entry).ok()
}

pub(crate) fn summarize_provider_lane_tool_request(
    turn: &ProviderTurn,
    turn_result: &TurnResult,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Option<String> {
    match turn_result {
        TurnResult::FinalText(_) | TurnResult::StreamingText(_) | TurnResult::StreamingDone(_) => {
            summarize_tool_followup_request(&turn.tool_intents)
        }
        TurnResult::NeedsApproval(_)
        | TurnResult::ToolDenied(_)
        | TurnResult::ToolError(_)
        | TurnResult::ProviderError(_) => summarize_failed_provider_lane_tool_request(turn, trace),
    }
}

pub(crate) fn summarize_failed_provider_lane_tool_request(
    turn: &ProviderTurn,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Option<String> {
    let failed_tool_call_id = trace.and_then(first_failed_provider_lane_tool_call_id);
    if let Some(failed_tool_call_id) = failed_tool_call_id {
        let failed_intent = turn
            .tool_intents
            .iter()
            .find(|intent| intent.tool_call_id == failed_tool_call_id)?;
        let summary = summarize_single_tool_followup_request(failed_intent);
        return summary;
    }

    match turn.tool_intents.as_slice() {
        [intent] => summarize_single_tool_followup_request(intent),
        [] => None,
        _ => summarize_tool_followup_request(&turn.tool_intents),
    }
}

fn first_failed_provider_lane_tool_call_id(trace: &ToolBatchExecutionTrace) -> Option<&str> {
    let failed_outcome = trace.intent_outcomes.iter().find(|intent_outcome| {
        !matches!(
            intent_outcome.status,
            ToolBatchExecutionIntentStatus::Completed
        )
    })?;
    let tool_call_id = failed_outcome.tool_call_id.as_str();
    Some(tool_call_id)
}
fn tool_followup_request_entry(intent: &ToolIntent) -> Value {
    let canonical_tool_name = effective_followup_tool_name(intent);
    let visible_tool_name = effective_followup_visible_tool_name(intent);
    let request = effective_followup_request(intent);
    let request = sanitize_followup_request_summary(canonical_tool_name.as_str(), request);
    serde_json::json!({
        "tool": visible_tool_name,
        "request": request,
    })
}

fn sanitize_followup_request_summary(tool_name: &str, request: Value) -> Value {
    crate::tools::summarize_tool_request_for_display(tool_name, request)
}

pub(crate) fn effective_followup_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    if canonical_tool_name != "tool.invoke" {
        return canonical_tool_name.to_owned();
    }

    if let Some((tool_name, _arguments)) =
        crate::tools::invoked_discoverable_tool_request(&intent.args_json)
    {
        return tool_name.to_owned();
    }

    intent
        .args_json
        .get("tool_id")
        .and_then(Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .unwrap_or(canonical_tool_name)
        .to_owned()
}

pub(crate) fn effective_followup_visible_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = effective_followup_tool_name(intent);
    crate::tools::user_visible_tool_name(canonical_tool_name.as_str())
}

pub(crate) fn effective_followup_request(intent: &ToolIntent) -> Value {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    if canonical_tool_name != "tool.invoke" {
        return crate::tools::normalize_shell_payload_for_request(
            canonical_tool_name,
            intent.args_json.clone(),
        );
    }

    let raw_tool_id = intent.args_json.get("tool_id").and_then(Value::as_str);
    let resolved_invoke = crate::tools::invoked_discoverable_tool_request(&intent.args_json);
    let (invoked_tool_name, request_payload) = match resolved_invoke {
        Some((tool_name, arguments)) => {
            let request_payload =
                strip_grouped_hidden_operation_from_request(raw_tool_id, arguments.clone());
            (tool_name, request_payload)
        }
        None => {
            let invoked_tool_name = raw_tool_id
                .map(crate::tools::canonical_tool_name)
                .unwrap_or(canonical_tool_name);
            let request_payload = intent
                .args_json
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| intent.args_json.clone());
            (invoked_tool_name, request_payload)
        }
    };

    crate::tools::normalize_shell_payload_for_request(invoked_tool_name, request_payload)
}

fn strip_grouped_hidden_operation_from_request(raw_tool_id: Option<&str>, request: Value) -> Value {
    let Some(raw_tool_id) = raw_tool_id.map(crate::tools::canonical_tool_name) else {
        return request;
    };
    let is_grouped_hidden_surface = crate::tools::is_tool_surface_id(raw_tool_id)
        && !crate::tools::is_provider_exposed_tool_name(raw_tool_id);
    if !is_grouped_hidden_surface {
        return request;
    }

    let Value::Object(mut request_object) = request else {
        return request;
    };
    request_object.remove("operation");
    Value::Object(request_object)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDrivenReplyBaseDecision {
    FinalizeDirect {
        reply: String,
    },
    RequireFollowup {
        raw_reply: String,
        payload: ToolDrivenFollowupPayload,
    },
}

impl ToolDrivenReplyBaseDecision {
    pub fn resolution_mode(&self) -> ReplyResolutionMode {
        match self {
            Self::FinalizeDirect { .. } => ReplyResolutionMode::Direct,
            Self::RequireFollowup { .. } => ReplyResolutionMode::CompletionPass,
        }
    }

    pub fn followup_kind(&self) -> Option<ToolDrivenFollowupKind> {
        match self {
            Self::FinalizeDirect { .. } => None,
            Self::RequireFollowup { payload, .. } => Some(payload.kind()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDrivenReplyPhase {
    raw_reply: Option<String>,
    decision: ToolDrivenReplyBaseDecision,
}

impl ToolDrivenReplyPhase {
    pub fn new(
        assistant_preface: &str,
        had_tool_intents: bool,
        raw_tool_output_requested: bool,
        turn_result: &TurnResult,
    ) -> Self {
        let kernel = ToolDrivenReplyKernel::new(assistant_preface, had_tool_intents, turn_result);
        Self {
            raw_reply: kernel.raw_reply(),
            decision: kernel.base_decision(raw_tool_output_requested),
        }
    }

    pub fn raw_reply(&self) -> Option<&str> {
        self.raw_reply.as_deref()
    }

    pub fn decision(&self) -> &ToolDrivenReplyBaseDecision {
        &self.decision
    }

    pub fn into_decision(self) -> ToolDrivenReplyBaseDecision {
        self.decision
    }

    pub fn resolution_mode(&self) -> ReplyResolutionMode {
        self.decision.resolution_mode()
    }

    pub fn followup_kind(&self) -> Option<ToolDrivenFollowupKind> {
        self.decision.followup_kind()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptMarker {
    ToolApprovalRequired,
    ApprovalRequired,
}

impl ApprovalPromptMarker {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToolApprovalRequired => "[tool_approval_required]",
            Self::ApprovalRequired => "[approval_required]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptLocale {
    En,
    Cjk,
}

impl ApprovalPromptLocale {
    pub const fn is_cjk(self) -> bool {
        matches!(self, Self::Cjk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptActionId {
    Yes,
    Auto,
    Full,
    Esc,
}

impl ApprovalPromptActionId {
    pub const fn command(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Esc => "esc",
        }
    }

    pub const fn numeric_alias(self) -> &'static str {
        match self {
            Self::Yes => "1",
            Self::Auto => "2",
            Self::Full => "3",
            Self::Esc => "4",
        }
    }

    pub const fn all() -> [Self; 4] {
        [Self::Yes, Self::Auto, Self::Full, Self::Esc]
    }

    fn matches_normalized_input(self, normalized: &str) -> bool {
        match self {
            Self::Yes => matches!(
                normalized,
                "1" | "y"
                    | "yes"
                    | "run"
                    | "once"
                    | "run once"
                    | "本次"
                    | "一次"
                    | "运行一次"
                    | "本次运行"
                    | "仅这次"
            ),
            Self::Auto => matches!(
                normalized,
                "2" | "auto" | "session auto" | "自动" | "本会话自动"
            ),
            Self::Full => matches!(
                normalized,
                "3" | "full"
                    | "full auto"
                    | "session full"
                    | "session full auto"
                    | "全自动"
                    | "本会话全自动"
            ),
            Self::Esc => matches!(
                normalized,
                "4" | "esc"
                    | "cancel"
                    | "skip"
                    | "skip call"
                    | "取消"
                    | "跳过"
                    | "跳过这次"
                    | "这次跳过"
                    | "不运行"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptActionEffect {
    CurrentCallOnly,
    SessionAuto,
    SessionFull,
    SkipCurrentCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptActionView {
    pub id: ApprovalPromptActionId,
    pub effect: ApprovalPromptActionEffect,
    pub command: String,
    pub numeric_alias: String,
    pub label: String,
    pub summary: String,
    #[serde(default)]
    pub detail_lines: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptView {
    pub marker: ApprovalPromptMarker,
    pub preface: Option<String>,
    pub tool_name: Option<String>,
    pub request_id: Option<String>,
    pub rule_id: Option<String>,
    pub reason: Option<String>,
    pub locale: ApprovalPromptLocale,
    #[serde(default)]
    pub actions: Vec<ApprovalPromptActionView>,
}

impl ApprovalPromptView {
    pub fn title(&self) -> Option<String> {
        match self.locale {
            ApprovalPromptLocale::Cjk => self
                .tool_name
                .as_ref()
                .map(|tool_name| format!("准备调用 {tool_name}"))
                .or_else(|| Some("工具调用需要确认".to_owned())),
            ApprovalPromptLocale::En => self
                .tool_name
                .as_ref()
                .map(|tool_name| format!("loong wants to call {tool_name}"))
                .or_else(|| Some("Tool call needs confirmation".to_owned())),
        }
    }

    pub fn pause_reason_title(&self) -> String {
        if self.locale.is_cjk() {
            "为什么停下来".to_owned()
        } else {
            "Why it paused".to_owned()
        }
    }

    pub fn request_section_title(&self) -> String {
        if self.locale.is_cjk() {
            "当前请求".to_owned()
        } else {
            "Pending request".to_owned()
        }
    }

    pub fn request_id_label(&self) -> String {
        if self.locale.is_cjk() {
            "请求 ID".to_owned()
        } else {
            "request id".to_owned()
        }
    }

    pub fn tool_label(&self) -> String {
        if self.locale.is_cjk() {
            "工具".to_owned()
        } else {
            "tool".to_owned()
        }
    }

    pub fn subtitle(&self) -> String {
        if self.locale.is_cjk() {
            "工具确认".to_owned()
        } else {
            "tool consent".to_owned()
        }
    }

    pub fn action_commands_text(&self) -> String {
        self.actions
            .iter()
            .map(|action| action.command.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn action_numeric_aliases_text(&self) -> String {
        self.actions
            .iter()
            .map(|action| action.numeric_alias.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn reply_hint_lines(&self) -> Vec<String> {
        if self.actions.is_empty() {
            return Vec::new();
        }

        let action_commands = self.action_commands_text();
        match self.locale {
            ApprovalPromptLocale::Cjk => vec![
                format!("可直接回复：{action_commands}"),
                self.actions
                    .iter()
                    .map(|action| format!("{}={}", action.command, action.summary))
                    .collect::<Vec<_>>()
                    .join("，"),
            ],
            ApprovalPromptLocale::En => vec![
                format!("Reply with: {action_commands}"),
                self.actions
                    .iter()
                    .map(|action| format!("{} = {}", action.command, action.summary))
                    .collect::<Vec<_>>()
                    .join(", "),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyPersistenceMode {
    Success,
    InlineProviderError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyResolutionMode {
    Direct,
    CompletionPass,
}

#[derive(Debug, Clone)]
pub enum ProviderTurnRequestAction {
    Continue { turn: ProviderTurn },
    FinalizeInlineProviderError { reply: String },
    ReturnError { error: String },
}

pub fn decide_provider_turn_request_action(
    result: CliResult<ProviderTurn>,
    error_mode: ProviderErrorMode,
) -> ProviderTurnRequestAction {
    match result {
        Ok(turn) => ProviderTurnRequestAction::Continue { turn },
        Err(error) => match error_mode {
            ProviderErrorMode::Propagate => ProviderTurnRequestAction::ReturnError { error },
            ProviderErrorMode::InlineMessage => {
                ProviderTurnRequestAction::FinalizeInlineProviderError {
                    reply: format_provider_error_reply(&error),
                }
            }
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillInvokeContext {
    pub skill_id: String,
    pub display_name: String,
    pub instructions: String,
    pub skill_root: Option<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub blocked_tools: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolDrivenReplyKernel<'a> {
    assistant_preface: &'a str,
    had_tool_intents: bool,
    turn_result: &'a TurnResult,
}

impl<'a> ToolDrivenReplyKernel<'a> {
    pub fn new(
        assistant_preface: &'a str,
        had_tool_intents: bool,
        turn_result: &'a TurnResult,
    ) -> Self {
        Self {
            assistant_preface,
            had_tool_intents,
            turn_result,
        }
    }

    pub fn fallback_reply(&self) -> String {
        compose_assistant_reply(
            self.assistant_preface,
            self.had_tool_intents,
            self.turn_result.clone(),
        )
    }

    pub fn raw_reply(&self) -> Option<String> {
        if !self.had_tool_intents {
            return None;
        }
        match self.turn_result {
            TurnResult::FinalText(text)
            | TurnResult::StreamingText(text)
            | TurnResult::StreamingDone(text) => {
                let sanitized_text = sanitize_reply_text(text);
                let reply =
                    join_non_empty_lines(&[self.assistant_preface, sanitized_text.as_str()]);
                Some(reply)
            }
            TurnResult::NeedsApproval(requirement) => Some(format_approval_required_reply(
                self.assistant_preface,
                requirement,
            )),
            TurnResult::ToolDenied(failure) | TurnResult::ToolError(failure) => {
                Some(join_non_empty_lines(&[
                    self.assistant_preface,
                    failure.reason.as_str(),
                ]))
            }
            TurnResult::ProviderError(_) => None,
        }
    }

    pub fn followup_payload(&self) -> Option<ToolDrivenFollowupPayload> {
        tool_driven_followup_payload(self.had_tool_intents, self.turn_result)
    }

    pub fn base_decision(&self, raw_tool_output_requested: bool) -> ToolDrivenReplyBaseDecision {
        let fallback_reply = self.fallback_reply();
        let Some(payload) = self.followup_payload() else {
            return ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: fallback_reply,
            };
        };
        let raw_reply = self.raw_reply().unwrap_or_else(|| fallback_reply.clone());
        let recovery_requires_followup =
            matches!(payload, ToolDrivenFollowupPayload::DiscoveryRecovery { .. });
        if raw_tool_output_requested && !recovery_requires_followup {
            ToolDrivenReplyBaseDecision::FinalizeDirect { reply: raw_reply }
        } else {
            ToolDrivenReplyBaseDecision::RequireFollowup { raw_reply, payload }
        }
    }
}

pub fn user_requested_raw_tool_output(user_input: &str) -> bool {
    let normalized = user_input.to_ascii_lowercase();
    let trimmed = normalized.trim();

    if trimmed == "[ok]" {
        return true;
    }

    let explicit_signals = [
        "raw tool output",
        "raw output",
        "exact output",
        "full output",
        "verbatim",
        "raw json",
        "raw payload",
        "full payload",
        "exact payload",
        "payload as json",
        "output as json",
    ];

    explicit_signals
        .iter()
        .any(|signal| normalized.contains(signal))
}

pub fn compose_assistant_reply(
    assistant_preface: &str,
    had_tool_intents: bool,
    turn_result: TurnResult,
) -> String {
    match turn_result {
        TurnResult::FinalText(text)
        | TurnResult::StreamingText(text)
        | TurnResult::StreamingDone(text) => {
            let sanitized_text = sanitize_reply_text(text.as_str());
            if had_tool_intents {
                join_non_empty_lines(&[assistant_preface, sanitized_text.as_str()])
            } else {
                sanitized_text
            }
        }
        TurnResult::NeedsApproval(requirement) => {
            format_approval_required_reply(assistant_preface, &requirement)
        }
        TurnResult::ToolDenied(failure) => {
            join_non_empty_lines(&[assistant_preface, failure.reason.as_str()])
        }
        TurnResult::ToolError(failure) => {
            join_non_empty_lines(&[assistant_preface, failure.reason.as_str()])
        }
        TurnResult::ProviderError(failure) => {
            let inline = format_provider_error_reply(failure.reason.as_str());
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
    }
}

pub fn format_approval_required_reply(
    assistant_preface: &str,
    requirement: &ApprovalRequirement,
) -> String {
    render_approval_prompt_view(&approval_prompt_view_from_requirement(
        assistant_preface,
        requirement,
    ))
}

pub fn parse_approval_prompt_view(assistant_text: &str) -> Option<ApprovalPromptView> {
    let (marker_index, marker) = find_approval_prompt_marker(assistant_text)?;
    let preface = trimmed_non_empty(assistant_text.get(..marker_index).unwrap_or_default());
    let body = assistant_text.get(marker_index..)?;
    let locale = approval_prompt_locale_from_text(assistant_text);
    let mut tool_name = None;
    let mut request_id = None;
    let mut rule_id = None;
    let mut reason = None;

    for line in body.lines() {
        if let Some(value) = line.strip_prefix("tool: ") {
            tool_name = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("request_id: ") {
            request_id = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("rule_id: ") {
            rule_id = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("reason: ") {
            reason = trimmed_non_empty(value);
        }
    }

    Some(ApprovalPromptView {
        marker,
        preface,
        tool_name,
        request_id,
        rule_id,
        reason,
        locale,
        actions: approval_prompt_actions(marker, locale),
    })
}

pub fn normalize_approval_prompt_control_input(input: &str) -> String {
    let compatibility = input.nfkc().collect::<String>();
    let trimmed = compatibility.trim().trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '`' | '"'
                    | '\''
                    | '.'
                    | ','
                    | ':'
                    | ';'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
            )
    });
    let lowercased = trimmed.to_lowercase();

    let normalized = lowercased
        .chars()
        .map(|character| match character {
            '_' | '-' => ' ',
            other => other,
        })
        .collect::<String>();

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_approval_prompt_action_input(input: &str) -> Option<ApprovalPromptActionId> {
    let normalized = normalize_approval_prompt_control_input(input);
    ApprovalPromptActionId::all()
        .into_iter()
        .find(|action| action.matches_normalized_input(normalized.as_str()))
}

fn render_approval_prompt_view(view: &ApprovalPromptView) -> String {
    let mut lines = Vec::new();
    lines.push(view.marker.as_str().to_owned());
    if let Some(tool_name) = view.tool_name.as_deref() {
        lines.push(format!("tool: {tool_name}"));
    }
    if let Some(request_id) = view.request_id.as_deref() {
        lines.push(format!("request_id: {request_id}"));
    }
    if let Some(rule_id) = view.rule_id.as_deref() {
        lines.push(format!("rule_id: {rule_id}"));
    }
    if let Some(reason) = view.reason.as_deref() {
        lines.push(format!("reason: {reason}"));
    }
    if !view.actions.is_empty() {
        lines.push(format!(
            "allowed_decisions: {}",
            view.action_commands_text()
        ));
        for action in &view.actions {
            for detail_line in &action.detail_lines {
                lines.push(detail_line.clone());
            }
        }
        lines.push(String::new());
        lines.extend(view.reply_hint_lines());
    }
    let body = lines.join("\n");
    join_non_empty_lines(&[view.preface.as_deref().unwrap_or_default(), body.as_str()])
}

fn approval_prompt_view_from_requirement(
    assistant_preface: &str,
    requirement: &ApprovalRequirement,
) -> ApprovalPromptView {
    let marker = match requirement.kind {
        ApprovalRequirementKind::GovernedTool => ApprovalPromptMarker::ToolApprovalRequired,
        ApprovalRequirementKind::KernelContextRequired => ApprovalPromptMarker::ApprovalRequired,
    };
    let locale = approval_prompt_locale_from_text(
        join_non_empty_lines(&[assistant_preface, requirement.reason.as_str()]).as_str(),
    );

    ApprovalPromptView {
        marker,
        preface: trimmed_non_empty(assistant_preface),
        tool_name: requirement
            .tool_name
            .as_deref()
            .map(crate::tools::user_visible_tool_name),
        request_id: requirement.approval_request_id.clone(),
        rule_id: trimmed_non_empty(requirement.rule_id.as_str()),
        reason: trimmed_non_empty(requirement.reason.as_str()),
        locale,
        actions: approval_prompt_actions(marker, locale),
    }
}

fn approval_prompt_actions(
    marker: ApprovalPromptMarker,
    locale: ApprovalPromptLocale,
) -> Vec<ApprovalPromptActionView> {
    if marker != ApprovalPromptMarker::ToolApprovalRequired {
        return Vec::new();
    }

    let make_action = |id,
                       effect,
                       label_cjk: &str,
                       label_en: &str,
                       summary_cjk: &str,
                       summary_en: &str,
                       detail_cjk: &[&str],
                       detail_en: &[&str],
                       recommended| ApprovalPromptActionView {
        id,
        effect,
        command: id.command().to_owned(),
        numeric_alias: id.numeric_alias().to_owned(),
        label: if locale.is_cjk() {
            label_cjk.to_owned()
        } else {
            label_en.to_owned()
        },
        summary: if locale.is_cjk() {
            summary_cjk.to_owned()
        } else {
            summary_en.to_owned()
        },
        detail_lines: if locale.is_cjk() {
            detail_cjk.iter().map(|line| (*line).to_owned()).collect()
        } else {
            detail_en.iter().map(|line| (*line).to_owned()).collect()
        },
        recommended,
    };

    vec![
        make_action(
            ApprovalPromptActionId::Yes,
            ApprovalPromptActionEffect::CurrentCallOnly,
            "本次运行",
            "Run once",
            "仅本次运行",
            "run once",
            &["只运行当前这次 tool call"],
            &["Execute only this tool call"],
            true,
        ),
        make_action(
            ApprovalPromptActionId::Auto,
            ApprovalPromptActionEffect::SessionAuto,
            "本会话自动",
            "Session auto",
            "本会话自动",
            "session auto mode",
            &[
                "后续低风险工具自动运行",
                "写文件、执行 shell、切换 provider 等仍会停下来",
            ],
            &[
                "Low-risk tools continue automatically",
                "Writes, shell exec, provider switching, and similar actions still pause",
            ],
            false,
        ),
        make_action(
            ApprovalPromptActionId::Full,
            ApprovalPromptActionEffect::SessionFull,
            "本会话全自动",
            "Session full-auto",
            "本会话全自动",
            "session full-auto mode",
            &[
                "本会话内不再询问 tool consent",
                "仍不会绕过 governed approval、shell allowlist 等硬限制",
            ],
            &[
                "Stop asking for tool consent in this session",
                "Governed approvals and kernel hard limits still apply",
            ],
            false,
        ),
        make_action(
            ApprovalPromptActionId::Esc,
            ApprovalPromptActionEffect::SkipCurrentCall,
            "跳过这次",
            "Skip call",
            "跳过这次",
            "skip this call",
            &["不执行这次 tool call"],
            &["Do not run this tool call"],
            false,
        ),
    ]
}

fn find_approval_prompt_marker(text: &str) -> Option<(usize, ApprovalPromptMarker)> {
    let tool_marker = text.find(ApprovalPromptMarker::ToolApprovalRequired.as_str());
    let generic_marker = text.find(ApprovalPromptMarker::ApprovalRequired.as_str());
    match (tool_marker, generic_marker) {
        (Some(tool_index), Some(generic_index)) if tool_index <= generic_index => {
            Some((tool_index, ApprovalPromptMarker::ToolApprovalRequired))
        }
        (Some(_tool_index), Some(generic_index)) => {
            Some((generic_index, ApprovalPromptMarker::ApprovalRequired))
        }
        (Some(tool_index), None) => Some((tool_index, ApprovalPromptMarker::ToolApprovalRequired)),
        (None, Some(generic_index)) => {
            Some((generic_index, ApprovalPromptMarker::ApprovalRequired))
        }
        (None, None) => None,
    }
}

fn approval_prompt_locale_from_text(text: &str) -> ApprovalPromptLocale {
    if contains_cjk_text(text) {
        ApprovalPromptLocale::Cjk
    } else {
        ApprovalPromptLocale::En
    }
}

fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn contains_cjk_text(text: &str) -> bool {
    text.chars().any(is_cjk_character)
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3040..=0x30ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xac00..=0xd7af
            | 0xf900..=0xfaff
    )
}

pub fn tool_result_contains_truncation_signal(tool_result_text: &str) -> bool {
    let normalized = tool_result_text.to_ascii_lowercase();
    normalized.contains("...(truncated ")
        || normalized.contains("... (truncated ")
        || normalized.contains("[tool_result_truncated]")
        || tool_result_text
            .lines()
            .any(line_contains_structured_truncation_signal)
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

#[cfg(test)]
#[allow(dead_code)]
pub fn build_tool_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
    _tool_request_summary: Option<&str>,
) -> String {
    build_tool_followup_user_prompt_with_context(
        user_input,
        loop_warning_reason,
        tool_result_text,
        rendered_tool_result_text,
        None,
    )
}

pub fn build_tool_followup_user_prompt_with_context(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
    extra_context: Option<&str>,
) -> String {
    let prompt =
        if followup_prompt_uses_discovery_guidance(tool_result_text, rendered_tool_result_text) {
            DISCOVERY_RESULT_FOLLOWUP_PROMPT
        } else {
            TOOL_FOLLOWUP_PROMPT
        };

    let mut sections = vec![prompt.to_owned()];
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient."
        ));
    }
    if followup_prompt_needs_truncation_hint(tool_result_text, rendered_tool_result_text) {
        sections.push(TOOL_TRUNCATION_HINT_PROMPT.to_owned());
    }
    if let Some(continuation_guidance) =
        proactive_followup_continuation_context(tool_result_text, rendered_tool_result_text)
    {
        sections.push(continuation_guidance);
    }
    if let Some(extra_context) = extra_context {
        sections.push(extra_context.to_owned());
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
}

fn combine_followup_extra_context(parts: &[Option<&str>]) -> Option<String> {
    let joined = parts
        .iter()
        .flatten()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    (!joined.is_empty()).then(|| joined.join("\n\n"))
}

fn proactive_followup_continuation_context(
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> Option<String> {
    let primary_context = tool_result_text.and_then(parse_tool_result_followup_context);
    let fallback_context = rendered_tool_result_text.and_then(parse_tool_result_followup_context);
    let tool_result_context = primary_context.or(fallback_context)?;
    let continuation = parse_tool_result_continuation(&tool_result_context.payload_json)?;
    Some(render_tool_result_continuation_guidance(&continuation))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolResultFollowupContext {
    payload_json: Value,
}

fn parse_tool_result_followup_context(tool_result_text: &str) -> Option<ToolResultFollowupContext> {
    tool_result_text.lines().find_map(|line| {
        let trimmed_line = line.trim();
        let tool_result_line = ToolResultLine::parse(trimmed_line)?;
        let payload_json = tool_result_line.payload_summary_json()?;
        Some(ToolResultFollowupContext { payload_json })
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolResultContinuation {
    state: String,
    is_terminal: bool,
    recommended_tool: Option<String>,
    recommended_payload: Option<Value>,
    note: Option<String>,
}

fn parse_tool_result_continuation(payload_json: &Value) -> Option<ToolResultContinuation> {
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

pub fn build_discovery_recovery_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    recovery_reason: &str,
) -> String {
    let mut sections = vec![DISCOVERY_RECOVERY_FOLLOWUP_PROMPT.to_owned()];
    sections.push(format!("Recovery reason:\n{recovery_reason}"));
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating identical unavailable tool calls. Search for the missing capability or change strategy."
        ));
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
}

fn followup_prompt_needs_truncation_hint(
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

fn followup_prompt_uses_discovery_guidance(
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

pub fn parse_external_skill_invoke_context(
    tool_result_text: &str,
) -> Option<ExternalSkillInvokeContext> {
    tool_result_text
        .trim()
        .lines()
        .filter_map(parse_external_skill_invoke_context_line)
        .next()
}

pub fn external_skill_invoke_context_from_payload_summary(
    payload_json: &Value,
) -> Option<ExternalSkillInvokeContext> {
    let instructions = payload_json
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let skill_id = payload_json
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("external-skill")
        .to_owned();
    let display_name = payload_json
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(skill_id.as_str())
        .to_owned();
    let skill_root = payload_json
        .get("skill_root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let metadata = payload_json.get("metadata").and_then(Value::as_object);
    let allowed_tools = metadata
        .and_then(|metadata| metadata.get("allowed_tools"))
        .map(parse_external_skill_tool_restrictions)
        .unwrap_or_default();
    let blocked_tools = metadata
        .and_then(|metadata| metadata.get("blocked_tools"))
        .map(parse_external_skill_tool_restrictions)
        .unwrap_or_default();
    Some(ExternalSkillInvokeContext {
        skill_id,
        display_name,
        instructions,
        skill_root,
        allowed_tools,
        blocked_tools,
    })
}

pub fn reduce_followup_payload_for_model<'a>(label: &str, text: &'a str) -> Cow<'a, str> {
    super::tool_result_reduction::reduce_followup_payload_for_model(label, text)
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

fn envelope_uses_external_skill_context(envelope: &Value) -> bool {
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
    let payload_semantics = envelope_payload_semantics(envelope);
    payload_semantics == Some(expected_semantics)
}

pub fn build_external_skill_system_message(skill_context: &ExternalSkillInvokeContext) -> String {
    format!(
        "External skill `{}` ({}) is now active for this task. Treat the following `SKILL.md` content as trusted runtime guidance until superseded.\n\n{}",
        skill_context.skill_id, skill_context.display_name, skill_context.instructions
    )
}

pub fn build_external_skill_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    skill_context: &ExternalSkillInvokeContext,
) -> String {
    let mut sections = vec![
        EXTERNAL_SKILL_FOLLOWUP_PROMPT.to_owned(),
        format!(
            "Loaded external skill:\n- id: {}\n- name: {}",
            skill_context.skill_id, skill_context.display_name
        ),
    ];
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient."
        ));
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
}

#[cfg(test)]
#[allow(dead_code)]
pub fn build_tool_result_followup_tail<F>(
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    build_tool_result_followup_tail_with_contract(
        assistant_preface,
        tool_result_text,
        user_input,
        loop_warning_reason,
        None,
        payload_mapper,
    )
}

pub(crate) fn build_tool_result_followup_tail_with_contract<F>(
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    continuation_contract: Option<&str>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    let mut messages = Vec::new();
    append_followup_preface(&mut messages, assistant_preface);
    if let Some(skill_context) = parse_external_skill_invoke_context(tool_result_text) {
        messages.push(serde_json::json!({
            "role": "system",
            "content": build_external_skill_system_message(&skill_context),
        }));
        append_followup_warning(&mut messages, loop_warning_reason);
        messages.push(serde_json::json!({
            "role": "user",
            "content": build_external_skill_followup_user_prompt(
                user_input,
                loop_warning_reason,
                &skill_context,
            ),
        }));
        return messages;
    }

    let label = ToolDrivenFollowupLabel::ToolResult;
    let bounded_result = payload_mapper(label.as_str(), tool_result_text);
    let assistant_content =
        ToolDrivenFollowupTextRef::new(label, bounded_result.as_str()).render_assistant_content();
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": assistant_content,
    }));
    append_followup_warning(&mut messages, loop_warning_reason);
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_tool_followup_user_prompt_with_context(
            user_input,
            loop_warning_reason,
            Some(tool_result_text),
            Some(bounded_result.as_str()),
            continuation_contract,
        ),
    }));
    messages
}

#[cfg(test)]
#[allow(dead_code)]
pub fn build_tool_failure_followup_tail<F>(
    assistant_preface: &str,
    tool_failure_reason: &str,
    tool_request_summary: Option<&str>,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    build_tool_failure_followup_tail_with_request_summary(
        assistant_preface,
        tool_failure_reason,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        payload_mapper,
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub fn build_tool_failure_followup_tail_with_request_summary<F>(
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    build_tool_failure_followup_tail_with_request_summary_and_contract(
        assistant_preface,
        tool_failure_reason,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        None,
        payload_mapper,
    )
}

pub(crate) fn build_tool_failure_followup_tail_with_request_summary_and_contract<F>(
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
    continuation_contract: Option<&str>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    let mut messages = Vec::new();
    append_followup_preface(&mut messages, assistant_preface);
    if let Some(tool_request_summary) = tool_request_summary {
        let bounded_request = payload_mapper("tool_request", tool_request_summary);
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": format!("[tool_request]\n{bounded_request}"),
        }));
    }
    let repair_guidance =
        render_tool_failure_repair_guidance(tool_failure_reason, tool_request_summary);
    let label = ToolDrivenFollowupLabel::ToolFailure;
    let bounded_failure = payload_mapper(label.as_str(), tool_failure_reason);
    let bounded_failure = if repair_guidance.is_some() {
        format!("tool input needs repair: {bounded_failure}")
    } else {
        bounded_failure
    };
    let assistant_content =
        ToolDrivenFollowupTextRef::new(label, bounded_failure.as_str()).render_assistant_content();
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": assistant_content,
    }));
    append_followup_warning(&mut messages, loop_warning_reason);
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_tool_followup_user_prompt_with_context(
            user_input,
            loop_warning_reason,
            None,
            None,
            combine_followup_extra_context(&[
                repair_guidance.as_deref(),
                continuation_contract,
            ])
            .as_deref(),
        ),
    }));
    messages
}

pub fn build_discovery_recovery_followup_tail<F>(
    assistant_preface: &str,
    recovery_reason: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    let mut messages = Vec::new();
    append_followup_preface(&mut messages, assistant_preface);
    let label = ToolDrivenFollowupLabel::DiscoveryRecovery;
    let bounded_recovery = payload_mapper(label.as_str(), recovery_reason);
    let assistant_content =
        ToolDrivenFollowupTextRef::new(label, bounded_recovery.as_str()).render_assistant_content();
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": assistant_content,
    }));
    append_followup_warning(&mut messages, loop_warning_reason);
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_discovery_recovery_followup_user_prompt(
            user_input,
            loop_warning_reason,
            bounded_recovery.as_str(),
        ),
    }));
    messages
}

#[cfg(test)]
#[allow(dead_code)]
pub fn build_tool_driven_followup_tail<F>(
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    tool_request_summary: Option<&str>,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    build_tool_driven_followup_tail_with_request_summary(
        assistant_preface,
        payload,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        payload_mapper,
    )
}

pub fn build_tool_driven_followup_tail_with_request_summary<F>(
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    build_tool_driven_followup_tail_with_request_summary_and_contract(
        assistant_preface,
        payload,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        None,
        payload_mapper,
    )
}

pub(crate) fn build_tool_driven_followup_tail_with_request_summary_and_contract<F>(
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
    continuation_contract: Option<&str>,
    payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    match payload {
        ToolDrivenFollowupPayload::ToolResult { text } => {
            build_tool_result_followup_tail_with_contract(
                assistant_preface,
                text.as_str(),
                user_input,
                loop_warning_reason,
                continuation_contract,
                payload_mapper,
            )
        }
        ToolDrivenFollowupPayload::ToolFailure { reason, .. } => {
            build_tool_failure_followup_tail_with_request_summary_and_contract(
                assistant_preface,
                reason.as_str(),
                user_input,
                loop_warning_reason,
                tool_request_summary,
                continuation_contract,
                payload_mapper,
            )
        }
        ToolDrivenFollowupPayload::DiscoveryRecovery { reason } => {
            build_discovery_recovery_followup_tail(
                assistant_preface,
                reason.as_str(),
                user_input,
                loop_warning_reason,
                payload_mapper,
            )
        }
    }
}

fn render_tool_failure_repair_guidance(
    tool_failure_reason: &str,
    tool_request_summary: Option<&str>,
) -> Option<String> {
    let tool_request_summary = tool_request_summary?;
    let request_summary_json = serde_json::from_str::<Value>(tool_request_summary).ok()?;
    let summary_tool_name = request_summary_json.get("tool").and_then(Value::as_str)?;
    let repair_tool_name = repair_guidance_tool_name(summary_tool_name, tool_failure_reason);
    let request_summary_request = request_summary_json.get("request");
    let direct_routing_guidance = render_direct_routing_failure_repair_guidance(
        repair_tool_name.as_str(),
        request_summary_request,
        tool_failure_reason,
    );

    if direct_routing_guidance.is_some() {
        return direct_routing_guidance;
    }

    let reason_mentions_repairable_shape = tool_failure_reason.contains("tool input needs repair")
        || tool_failure_reason.contains("payload must be an object")
        || tool_failure_reason.contains("payload.");

    if !reason_mentions_repairable_shape {
        return None;
    }

    let shell_guidance = render_shell_failure_repair_guidance(
        repair_tool_name.as_str(),
        request_summary_request,
        tool_failure_reason,
    );

    if shell_guidance.is_some() {
        return shell_guidance;
    }

    let guidance_from_request =
        render_tool_input_repair_guidance(repair_tool_name.as_str(), request_summary_request);

    if guidance_from_request.is_some() {
        return guidance_from_request;
    }

    render_tool_input_repair_guidance_from_reason(repair_tool_name.as_str(), tool_failure_reason)
}

fn render_direct_routing_failure_repair_guidance(
    tool_name: &str,
    request_summary_request: Option<&Value>,
    tool_failure_reason: &str,
) -> Option<String> {
    let normalized_reason = tool_failure_reason
        .strip_prefix("tool execution failed: ")
        .unwrap_or(tool_failure_reason);

    let guidance = if normalized_reason.starts_with("hidden_agent_requires_operation:") {
        "Add `operation` for grouped agent/runtime control requests such as session archive, cancel, recover, or approval workflows.".to_owned()
    } else if normalized_reason.starts_with("hidden_agent_requires_actionable_fields:") {
        "Add the concrete session / approval / delegate / provider / config fields needed for the request, or set `operation` when the grouped `tool.invoke` request is ambiguous.".to_owned()
    } else if normalized_reason.starts_with("hidden_skills_requires_actionable_fields:") {
        "Add search, inspect, install, run, or list fields for the grouped `skills` surface, or provide `operation` to make the request explicit.".to_owned()
    } else if normalized_reason.starts_with("hidden_channel_requires_operation:") {
        "Add `operation` for the grouped channel surface, for example `messages.send`, `messages.reply`, `card.update`, or `feishu.whoami`.".to_owned()
    } else {
        return None;
    };

    let visible_tool_name = repair_guidance_visible_tool_name(tool_name);
    let request_preview = request_summary_request
        .and_then(|request| serde_json::to_string(request).ok())
        .unwrap_or_else(|| "{}".to_owned());

    Some(format!(
        "Repair guidance for {visible_tool_name}:\n{guidance}\nCurrent request preview: {request_preview}"
    ))
}

fn repair_guidance_tool_name(summary_tool_name: &str, tool_failure_reason: &str) -> String {
    let trimmed_reason = tool_failure_reason.trim();
    let stripped_reason = trimmed_reason
        .strip_prefix("tool_preflight_denied: tool input needs repair: ")
        .or_else(|| trimmed_reason.strip_prefix("tool input needs repair: "))
        .unwrap_or(trimmed_reason);

    if let Some((tool_name, _)) = stripped_reason.split_once(" payload.") {
        return tool_name.to_owned();
    }

    if let Some(tool_name) = stripped_reason.strip_suffix(" payload must be an object") {
        return tool_name.to_owned();
    }

    crate::tools::canonical_tool_name(summary_tool_name).to_owned()
}

fn render_shell_failure_repair_guidance(
    tool_name: &str,
    request_summary_request: Option<&Value>,
    tool_failure_reason: &str,
) -> Option<String> {
    if crate::tools::user_visible_tool_name(tool_name) != "exec" {
        return None;
    }

    let request_object = request_summary_request?.as_object()?;
    let command = request_object.get("command").and_then(Value::as_str)?;
    let has_path_separator = command.contains('/') || command.contains('\\');
    let mentions_payload_command = tool_failure_reason.contains("payload.command");
    let mentions_path_separator = tool_failure_reason.contains("path separators");
    let should_render_guidance =
        has_path_separator || mentions_payload_command || mentions_path_separator;

    if !should_render_guidance {
        return None;
    }

    let bare_command = suggested_shell_command_name(command);
    let visible_tool_name = repair_guidance_visible_tool_name(tool_name);
    let guidance = format!(
        "Repair guidance for {visible_tool_name}:\nUse a bare lowercase executable name in `payload.command`.\nThe failed request used `{command}`; retry with `{bare_command}`."
    );
    Some(guidance)
}

fn suggested_shell_command_name(command: &str) -> String {
    let candidate = first_shell_command_segment(command).trim();
    let candidate = if !candidate.contains('/') && !candidate.contains('\\') {
        candidate.split_whitespace().next().unwrap_or(candidate)
    } else {
        candidate
    };
    candidate
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(candidate)
        .to_ascii_lowercase()
}

fn first_shell_command_segment(command: &str) -> &str {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix('"')
        && let Some((quoted, _)) = rest.split_once('"')
    {
        return quoted;
    }
    if let Some(rest) = trimmed.strip_prefix('\'')
        && let Some((quoted, _)) = rest.split_once('\'')
    {
        return quoted;
    }
    trimmed.split_whitespace().next().unwrap_or(trimmed)
}

pub fn build_tool_loop_guard_tail<F>(
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<ToolDrivenFollowupTextRef<'_>>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(ToolDrivenFollowupLabel, &str) -> String,
{
    let mut messages = Vec::new();
    let mut original_tool_result_text = None;
    let mut rendered_tool_result_text = Option::<String>::None;
    append_followup_preface(&mut messages, assistant_preface);
    if let Some(latest_tool_context) = latest_tool_context {
        let label = latest_tool_context.label();
        let text = latest_tool_context.text();
        let bounded = payload_mapper(label, text);
        let assistant_content =
            ToolDrivenFollowupTextRef::new(label, bounded.as_str()).render_assistant_content();
        if label == ToolDrivenFollowupLabel::ToolResult {
            original_tool_result_text = Some(text);
            rendered_tool_result_text = Some(bounded);
        }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
        }));
    }
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": format!("[tool_loop_guard]\n{reason}"),
    }));
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_tool_loop_guard_prompt(
            user_input,
            reason,
            original_tool_result_text,
            rendered_tool_result_text.as_deref(),
        ),
    }));
    messages
}

pub async fn request_completion_with_raw_fallback_detailed<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    messages: &[Value],
    binding: ConversationRuntimeBinding<'_>,
    raw_reply: &str,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> ParsedToolDrivenContinuationReply {
    match runtime
        .request_completion_with_retry_progress(config, messages, binding, retry_progress)
        .await
    {
        Ok(final_reply) => {
            let parsed_reply = parse_tool_driven_continuation_reply(final_reply.as_str());
            if parsed_reply.reply.is_empty() && parsed_reply.state.is_none() {
                parse_tool_driven_continuation_reply(raw_reply)
            } else if parsed_reply.reply.is_empty() {
                ParsedToolDrivenContinuationReply {
                    state: parsed_reply.state,
                    reply: sanitize_reply_text(raw_reply),
                }
            } else {
                parsed_reply
            }
        }
        Err(_) => parse_tool_driven_continuation_reply(raw_reply),
    }
}

pub async fn request_completion_with_raw_fallback<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    messages: &[Value],
    binding: ConversationRuntimeBinding<'_>,
    raw_reply: &str,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> String {
    request_completion_with_raw_fallback_detailed(
        runtime,
        config,
        messages,
        binding,
        raw_reply,
        retry_progress,
    )
    .await
    .reply
}

pub fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_followup_preface(messages: &mut Vec<Value>, assistant_preface: &str) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": preface,
        }));
    }
}

fn append_followup_warning(messages: &mut Vec<Value>, loop_warning_reason: Option<&str>) {
    if let Some(reason) = loop_warning_reason {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
}

fn build_tool_loop_guard_prompt(
    user_input: &str,
    reason: &str,
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> String {
    let mut sections = vec![
        TOOL_LOOP_GUARD_PROMPT.to_owned(),
        format!("Loop guard reason:\n{reason}"),
    ];
    if followup_prompt_needs_truncation_hint(tool_result_text, rendered_tool_result_text) {
        sections.push(TOOL_TRUNCATION_HINT_PROMPT.to_owned());
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
}

fn parse_external_skill_invoke_context_line(line: &str) -> Option<ExternalSkillInvokeContext> {
    let tool_result_line = ToolResultLine::parse(line)?;
    let envelope = serde_json::to_value(tool_result_line.envelope()).ok()?;
    let uses_external_skill_context = envelope_uses_external_skill_context(&envelope);
    let uses_legacy_carrier = tool_result_line.tool_name() == "external_skills.invoke";
    if !uses_legacy_carrier && !uses_external_skill_context {
        return None;
    }
    if tool_result_line.payload_truncated() {
        return None;
    }
    let payload_json = tool_result_line.payload_summary_json()?;
    external_skill_invoke_context_from_payload_summary(&payload_json)
}

fn parse_external_skill_tool_restrictions(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
pub(crate) fn parse_tool_result_followup_for_test(messages: &[Value]) -> (Value, Value) {
    let assistant_tool_result = messages
        .iter()
        .find(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .and_then(ToolDrivenFollowupMessageOwned::parse_assistant_content)
        .expect("assistant tool_result followup message should exist");
    assert_eq!(
        assistant_tool_result.label(),
        ToolDrivenFollowupLabel::ToolResult
    );
    let tool_result_line = ToolResultLine::parse(assistant_tool_result.body())
        .expect("tool result line should preserve structured envelope");
    let envelope = serde_json::to_value(tool_result_line.envelope())
        .expect("tool result envelope should serialize");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("payload summary should stay valid json");
    (envelope, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::turn_engine::{
        ApprovalRequirement, ApprovalRequirementKind, TurnFailure, TurnResult,
    };
    use serde_json::json;

    #[test]
    fn raw_tool_output_detection_keeps_known_signals() {
        assert!(user_requested_raw_tool_output("show raw tool output"));
        assert!(user_requested_raw_tool_output("give exact output as JSON"));
        assert!(!user_requested_raw_tool_output(
            "summarize the result briefly"
        ));
    }

    #[test]
    fn raw_tool_output_detection_ignores_payload_mentions_without_output_request() {
        assert!(!user_requested_raw_tool_output(
            "Callback hints mention the payload JSON, but just summarize the action."
        ));
        assert!(!user_requested_raw_tool_output(
            "The card callback token stays in internal payload context."
        ));
        assert!(user_requested_raw_tool_output(
            "Return the payload as JSON."
        ));
    }

    #[test]
    fn raw_tool_output_detection_ignores_generic_json_and_tool_output_requests() {
        assert!(!user_requested_raw_tool_output("summarize the tool output"));
        assert!(!user_requested_raw_tool_output("answer in json"));
        assert!(!user_requested_raw_tool_output("format the result as json"));
        assert!(user_requested_raw_tool_output("[ok]"));
    }

    #[test]
    fn compose_assistant_reply_keeps_tool_error_inline_reason() {
        let reply = compose_assistant_reply(
            "preface",
            true,
            TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure")),
        );
        assert_eq!(reply, "preface\ntemporary failure");
    }

    #[test]
    fn compose_assistant_reply_formats_governed_tool_approval_requirement() {
        let reply = compose_assistant_reply(
            "preface",
            true,
            TurnResult::NeedsApproval(ApprovalRequirement {
                kind: ApprovalRequirementKind::GovernedTool,
                reason: "operator approval required for governed tool".to_owned(),
                rule_id: "governed_tool_requires_approval".to_owned(),
                tool_name: Some("delegate_async".to_owned()),
                approval_key: Some("tool:delegate_async".to_owned()),
                approval_request_id: Some("apr_123".to_owned()),
            }),
        );

        assert!(reply.contains("[tool_approval_required]"));
        assert!(reply.contains("delegate_async"));
        assert!(reply.contains("apr_123"));
        assert!(reply.contains("yes"));
        assert!(reply.contains("auto"));
        assert!(reply.contains("full"));
        assert!(reply.contains("esc"));
    }

    #[test]
    fn parse_approval_prompt_view_recovers_localized_action_contract() {
        let reply = format_approval_required_reply(
            "我准备调用 provider.switch 来切换后续会话的 provider。",
            &ApprovalRequirement {
                kind: ApprovalRequirementKind::GovernedTool,
                reason: "`provider.switch` is not eligible for auto mode and needs operator confirmation"
                    .to_owned(),
                rule_id: "session_tool_consent_auto_blocked".to_owned(),
                tool_name: Some("provider.switch".to_owned()),
                approval_key: Some("tool:provider.switch".to_owned()),
                approval_request_id: Some("apr_provider_switch".to_owned()),
            },
        );

        let parsed = parse_approval_prompt_view(reply.as_str()).expect("parse approval prompt");
        assert_eq!(parsed.marker, ApprovalPromptMarker::ToolApprovalRequired);
        assert_eq!(
            parsed.preface.as_deref(),
            Some("我准备调用 provider.switch 来切换后续会话的 provider。")
        );
        assert_eq!(parsed.tool_name.as_deref(), Some("provider.switch"));
        assert_eq!(parsed.request_id.as_deref(), Some("apr_provider_switch"));
        assert_eq!(
            parsed.rule_id.as_deref(),
            Some("session_tool_consent_auto_blocked")
        );
        assert_eq!(parsed.locale, ApprovalPromptLocale::Cjk);
        assert_eq!(
            parsed
                .actions
                .iter()
                .map(|action| action.command.as_str())
                .collect::<Vec<_>>(),
            vec!["yes", "auto", "full", "esc"]
        );
        assert_eq!(
            parsed
                .actions
                .iter()
                .map(|action| action.label.as_str())
                .collect::<Vec<_>>(),
            vec!["本次运行", "本会话自动", "本会话全自动", "跳过这次"]
        );
    }

    #[test]
    fn approval_prompt_action_input_parser_accepts_skip_and_localized_aliases() {
        assert_eq!(
            parse_approval_prompt_action_input("run once"),
            Some(ApprovalPromptActionId::Yes)
        );
        assert_eq!(
            parse_approval_prompt_action_input("session full-auto"),
            Some(ApprovalPromptActionId::Full)
        );
        assert_eq!(
            parse_approval_prompt_action_input("跳过这次"),
            Some(ApprovalPromptActionId::Esc)
        );
        assert_eq!(
            parse_approval_prompt_action_input("skip call"),
            Some(ApprovalPromptActionId::Esc)
        );
        assert_eq!(parse_approval_prompt_action_input("maybe"), None);
    }

    #[test]
    fn approval_prompt_action_input_parser_accepts_full_width_aliases() {
        assert_eq!(
            parse_approval_prompt_action_input("ｙｅｓ"),
            Some(ApprovalPromptActionId::Yes)
        );
        assert_eq!(
            parse_approval_prompt_action_input("３"),
            Some(ApprovalPromptActionId::Full)
        );
        assert_eq!(
            parse_approval_prompt_action_input("ｓｋｉｐ　ｃａｌｌ"),
            Some(ApprovalPromptActionId::Esc)
        );
    }

    #[test]
    fn compose_assistant_reply_strips_think_tags_from_final_text() {
        let reply = compose_assistant_reply(
            "preface",
            false,
            TurnResult::FinalText("<think>internal reasoning</think>visible reply".to_owned()),
        );

        assert_eq!(reply, "visible reply");
    }

    #[test]
    fn tool_driven_reply_kernel_extracts_raw_reply_and_result_followup() {
        let result = TurnResult::FinalText("tool output".to_owned());
        let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

        assert_eq!(kernel.fallback_reply(), "preface\ntool output");
        assert_eq!(kernel.raw_reply(), Some("preface\ntool output".to_owned()));
        assert_eq!(
            kernel.followup_payload(),
            Some(ToolDrivenFollowupPayload::ToolResult {
                text: "tool output".to_owned(),
            })
        );
    }

    #[test]
    fn tool_driven_reply_kernel_strips_think_tags_from_raw_reply() {
        let result = TurnResult::FinalText(
            "<think>internal reasoning</think>visible tool output".to_owned(),
        );
        let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

        assert_eq!(
            kernel.raw_reply(),
            Some("preface\nvisible tool output".to_owned())
        );
    }

    #[test]
    fn tool_driven_reply_kernel_extracts_raw_reply_and_failure_followup() {
        let result =
            TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
        let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

        assert_eq!(kernel.fallback_reply(), "preface\ntemporary failure");
        assert_eq!(
            kernel.raw_reply(),
            Some("preface\ntemporary failure".to_owned())
        );
        assert_eq!(
            kernel.followup_payload(),
            Some(ToolDrivenFollowupPayload::ToolFailure {
                reason: "temporary failure".to_owned(),
                retryable: true,
            })
        );
    }

    #[test]
    fn tool_driven_reply_kernel_rejects_non_tool_followup_paths() {
        let provider_error = TurnResult::ProviderError(TurnFailure::provider(
            "provider_error",
            "provider unavailable",
        ));
        let kernel = ToolDrivenReplyKernel::new("preface", true, &provider_error);
        assert_eq!(kernel.raw_reply(), None);
        assert_eq!(kernel.followup_payload(), None);

        let plain_text = TurnResult::FinalText("plain reply".to_owned());
        let non_tool_kernel = ToolDrivenReplyKernel::new("preface", false, &plain_text);
        assert_eq!(non_tool_kernel.raw_reply(), None);
        assert_eq!(non_tool_kernel.followup_payload(), None);
        assert_eq!(non_tool_kernel.fallback_reply(), "plain reply");
    }

    #[test]
    fn tool_driven_followup_payload_reports_result_kind_and_context() {
        let payload = ToolDrivenFollowupPayload::ToolResult {
            text: "tool output".to_owned(),
        };
        let message_context = payload.message_context();

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolResult);
        assert_eq!(message_context.label(), ToolDrivenFollowupLabel::ToolResult);
        assert_eq!(message_context.label().as_str(), "tool_result");
        assert_eq!(message_context.text(), "tool output");
    }

    #[test]
    fn tool_driven_followup_payload_strips_think_tags_from_tool_result_text() {
        let turn_result = TurnResult::FinalText(
            "<think>internal reasoning</think>visible tool output".to_owned(),
        );

        assert_eq!(
            tool_driven_followup_payload(true, &turn_result),
            Some(ToolDrivenFollowupPayload::ToolResult {
                text: "visible tool output".to_owned(),
            })
        );
    }

    #[test]
    fn tool_driven_followup_payload_reports_failure_kind_and_context() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool failed".to_owned(),
            retryable: false,
        };
        let message_context = payload.message_context();

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolFailure);
        assert_eq!(
            message_context.label(),
            ToolDrivenFollowupLabel::ToolFailure
        );
        assert_eq!(message_context.label().as_str(), "tool_failure");
        assert_eq!(message_context.text(), "tool failed");
        assert!(!payload.retryable_failure());
    }

    #[test]
    fn tool_driven_followup_payload_reports_discovery_recovery_context() {
        let payload = ToolDrivenFollowupPayload::DiscoveryRecovery {
            reason: "tool_not_found: requested tool is not available".to_owned(),
        };
        let message_context = payload.message_context();

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::DiscoveryRecovery);
        assert_eq!(
            message_context.label(),
            ToolDrivenFollowupLabel::DiscoveryRecovery
        );
        assert_eq!(message_context.label().as_str(), "tool_recovery");
        assert_eq!(
            message_context.text(),
            "tool_not_found: requested tool is not available"
        );
    }

    #[test]
    fn tool_driven_followup_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_value(ToolDrivenFollowupKind::ToolResult).expect("serialize kind"),
            Value::String("tool_result".to_owned())
        );
        assert_eq!(
            serde_json::to_value(ToolDrivenFollowupKind::ToolFailure).expect("serialize kind"),
            Value::String("tool_failure".to_owned())
        );
        assert_eq!(
            serde_json::to_value(ToolDrivenFollowupKind::DiscoveryRecovery)
                .expect("serialize kind"),
            Value::String("discovery_recovery".to_owned())
        );
    }

    #[test]
    fn parse_tool_driven_continuation_reply_extracts_leading_marker() {
        let parsed = parse_tool_driven_continuation_reply(
            "<think>hidden</think>\n[followup_state:continue]\nNow merging the files.",
        );

        assert_eq!(parsed.state, Some(ToolDrivenContinuationState::Continue));
        assert_eq!(parsed.reply, "Now merging the files.");
        assert_eq!(
            sanitize_reply_text("[followup_state:done]\nFinished."),
            "Finished."
        );
    }

    #[test]
    fn parse_tool_driven_continuation_reply_leaves_unknown_marker_text_intact() {
        let parsed =
            parse_tool_driven_continuation_reply("[followup_state:unknown]\nkeep this literal");

        assert_eq!(parsed.state, None);
        assert_eq!(parsed.reply, "[followup_state:unknown]\nkeep this literal");
    }

    #[test]
    fn render_tool_followup_continuation_contract_includes_repair_and_retryable_context() {
        let repair = render_tool_followup_continuation_contract(
            ToolDrivenFollowupContractMode::RepairRetryableFailure,
        );

        assert!(repair.contains("[followup_state:continue]"));
        assert!(repair.contains("[followup_state:done]"));
        assert!(repair.contains("[followup_state:blocked]"));
        assert!(repair.contains("Do not describe the plan again"));
        assert!(repair.contains("retryable"));
    }

    #[test]
    fn missing_tool_call_followup_detects_pseudo_tool_commands() {
        let payload = missing_tool_call_followup_payload(
            "/workspace:df -h\n/tool.search:disk usage\n/web:disk usage command line",
        )
        .expect("pseudo-tool lines should trigger missing-tool-call recovery");

        let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
            panic!("expected tool failure payload");
        };

        assert!(reason.contains("pseudo-tool text"));
        assert!(reason.contains("Reply excerpt"));
        assert!(retryable);
    }

    #[test]
    fn missing_tool_call_followup_detects_empty_followup() {
        let payload = missing_tool_call_followup_payload("   ")
            .expect("empty followup should trigger missing-tool-call recovery");

        let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
            panic!("expected tool failure payload");
        };

        assert!(reason.contains("without any content or tool call"));
        assert!(retryable);
    }

    #[test]
    fn missing_tool_call_followup_ignores_normal_final_answer_text() {
        let payload = missing_tool_call_followup_payload(
            "The disk is nearly full because the cache directory is consuming most of the space.",
        );

        assert!(payload.is_none());
    }

    #[test]
    fn turn_failure_supports_discovery_recovery_requires_structured_metadata() {
        let recovery_failure = TurnFailure::policy_denied_with_discovery_recovery(
            "invalid_tool_lease",
            "tool.invoke needs a fresh lease from the current tool.search result. If you need a non-core capability, call tool.search with a short natural-language description of the task.",
        );
        let plain_failure = TurnFailure::policy_denied(
            "invalid_tool_lease",
            "tool execution failed: invalid_tool_lease: expired lease",
        );

        assert!(turn_failure_supports_discovery_recovery(&recovery_failure));
        assert!(!turn_failure_supports_discovery_recovery(&plain_failure));
    }

    #[test]
    fn tool_result_line_roundtrip_preserves_envelope() {
        let envelope = ToolResultEnvelope {
            status: "ok".to_owned(),
            tool: "file.read".to_owned(),
            tool_call_id: "call-1".to_owned(),
            payload_semantics: None,
            payload_summary: json!({
                "path": "README.md",
                "content": "hello"
            })
            .to_string(),
            payload_chars: 42,
            payload_truncated: false,
        };
        let tool_result_line = ToolResultLine::new("ok", envelope.clone());
        let rendered = tool_result_line.render().expect("render tool result line");
        let reparsed = ToolResultLine::parse(rendered.as_str()).expect("parse tool result line");

        assert_eq!(reparsed.envelope(), &envelope);
        assert_eq!(reparsed.tool_name(), "file.read");
        assert!(!reparsed.payload_truncated());
    }

    #[test]
    fn tool_driven_followup_message_owned_parses_typed_assistant_marker() {
        let message = ToolDrivenFollowupTextRef::new(
            ToolDrivenFollowupLabel::DiscoveryRecovery,
            "tool_not_found: requested tool is not available",
        )
        .render_assistant_content();
        let parsed = ToolDrivenFollowupMessageOwned::parse_assistant_content(message.as_str())
            .expect("parse assistant followup content");

        assert_eq!(parsed.label(), ToolDrivenFollowupLabel::DiscoveryRecovery);
        assert_eq!(
            parsed.body(),
            "tool_not_found: requested tool is not available"
        );
    }

    #[test]
    fn reply_resolution_mode_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_value(ReplyResolutionMode::Direct).expect("serialize mode"),
            Value::String("direct".to_owned())
        );
        assert_eq!(
            serde_json::to_value(ReplyResolutionMode::CompletionPass).expect("serialize mode"),
            Value::String("completion_pass".to_owned())
        );
    }

    #[test]
    fn tool_driven_reply_kernel_base_decision_finalizes_non_tool_reply_directly() {
        let result = TurnResult::FinalText("plain reply".to_owned());
        let kernel = ToolDrivenReplyKernel::new("preface", false, &result);

        assert_eq!(
            kernel.base_decision(false),
            ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "plain reply".to_owned(),
            }
        );
    }

    #[test]
    fn tool_driven_reply_kernel_base_decision_honors_raw_tool_output_mode() {
        let result = TurnResult::FinalText("tool output".to_owned());
        let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

        assert_eq!(
            kernel.base_decision(true),
            ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "preface\ntool output".to_owned(),
            }
        );
    }

    #[test]
    fn tool_driven_reply_kernel_base_decision_requires_followup_for_tool_failure() {
        let result =
            TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
        let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

        assert_eq!(
            kernel.base_decision(false),
            ToolDrivenReplyBaseDecision::RequireFollowup {
                raw_reply: "preface\ntemporary failure".to_owned(),
                payload: ToolDrivenFollowupPayload::ToolFailure {
                    reason: "temporary failure".to_owned(),
                    retryable: true,
                },
            }
        );
    }

    #[test]
    fn tool_driven_reply_base_decision_reports_followup_kind_only_for_followup_paths() {
        let direct = ToolDrivenReplyBaseDecision::FinalizeDirect {
            reply: "reply".to_owned(),
        };
        let followup = ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply: "raw".to_owned(),
            payload: ToolDrivenFollowupPayload::ToolResult {
                text: "tool output".to_owned(),
            },
        };

        assert_eq!(direct.resolution_mode(), ReplyResolutionMode::Direct);
        assert_eq!(
            followup.resolution_mode(),
            ReplyResolutionMode::CompletionPass
        );
        assert_eq!(direct.followup_kind(), None);
        assert_eq!(
            followup.followup_kind(),
            Some(ToolDrivenFollowupKind::ToolResult)
        );
    }

    #[test]
    fn tool_driven_reply_phase_finalizes_non_tool_reply_directly() {
        let result = TurnResult::FinalText("plain reply".to_owned());
        let phase = ToolDrivenReplyPhase::new("preface", false, false, &result);

        assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
        assert_eq!(phase.followup_kind(), None);
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "plain reply".to_owned(),
            }
        );
    }

    #[test]
    fn tool_driven_reply_phase_requires_followup_for_tool_success() {
        let result = TurnResult::FinalText("tool output".to_owned());
        let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

        assert_eq!(phase.resolution_mode(), ReplyResolutionMode::CompletionPass);
        assert_eq!(
            phase.followup_kind(),
            Some(ToolDrivenFollowupKind::ToolResult)
        );
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::RequireFollowup {
                raw_reply: "preface\ntool output".to_owned(),
                payload: ToolDrivenFollowupPayload::ToolResult {
                    text: "tool output".to_owned(),
                },
            }
        );
    }

    #[test]
    fn tool_driven_reply_phase_requires_followup_for_tool_failure() {
        let result =
            TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
        let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

        assert_eq!(phase.resolution_mode(), ReplyResolutionMode::CompletionPass);
        assert_eq!(
            phase.followup_kind(),
            Some(ToolDrivenFollowupKind::ToolFailure)
        );
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::RequireFollowup {
                raw_reply: "preface\ntemporary failure".to_owned(),
                payload: ToolDrivenFollowupPayload::ToolFailure {
                    reason: "temporary failure".to_owned(),
                    retryable: true,
                },
            }
        );
    }

    #[test]
    fn tool_driven_reply_phase_finalizes_approval_requirement_directly() {
        let result = TurnResult::NeedsApproval(ApprovalRequirement {
            kind: ApprovalRequirementKind::GovernedTool,
            reason: "operator approval required for governed tool".to_owned(),
            rule_id: "governed_tool_requires_approval".to_owned(),
            tool_name: Some("delegate_async".to_owned()),
            approval_key: Some("tool:delegate_async".to_owned()),
            approval_request_id: Some("apr_direct".to_owned()),
        });
        let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

        assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
        assert_eq!(phase.followup_kind(), None);
        assert_eq!(
            phase.raw_reply(),
            Some(
                "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: yes / auto / full / esc\nExecute only this tool call\nLow-risk tools continue automatically\nWrites, shell exec, provider switching, and similar actions still pause\nStop asking for tool consent in this session\nGoverned approvals and kernel hard limits still apply\nDo not run this tool call\n\nReply with: yes / auto / full / esc\nyes = run once, auto = session auto mode, full = session full-auto mode, esc = skip this call"
            )
        );
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: yes / auto / full / esc\nExecute only this tool call\nLow-risk tools continue automatically\nWrites, shell exec, provider switching, and similar actions still pause\nStop asking for tool consent in this session\nGoverned approvals and kernel hard limits still apply\nDo not run this tool call\n\nReply with: yes / auto / full / esc\nyes = run once, auto = session auto mode, full = session full-auto mode, esc = skip this call".to_owned(),
            }
        );
    }

    #[test]
    fn tool_driven_reply_phase_exposes_raw_reply_for_tool_success() {
        let result = TurnResult::FinalText("tool output".to_owned());
        let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

        assert_eq!(phase.raw_reply(), Some("preface\ntool output"));
    }

    #[test]
    fn tool_driven_reply_phase_exposes_raw_reply_for_tool_failure() {
        let result =
            TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
        let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

        assert_eq!(phase.raw_reply(), Some("preface\ntemporary failure"));
    }

    #[test]
    fn tool_driven_reply_phase_omits_raw_reply_for_non_tool_paths() {
        let result = TurnResult::FinalText("plain reply".to_owned());
        let phase = ToolDrivenReplyPhase::new("preface", false, false, &result);

        assert_eq!(phase.raw_reply(), None);
    }

    #[test]
    fn tool_driven_reply_phase_raw_mode_bypasses_completion_pass() {
        let result = TurnResult::FinalText("tool output".to_owned());
        let phase = ToolDrivenReplyPhase::new("preface", true, true, &result);

        assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
        assert_eq!(phase.followup_kind(), None);
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "preface\ntool output".to_owned(),
            }
        );
    }

    #[test]
    fn tool_result_followup_tail_promotes_external_skill_without_payload_mapping() {
        let tail = build_tool_result_followup_tail(
            "preface",
            r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#,
            "summarize note.md",
            Some("warning"),
            |_, _| panic!("external skill payload should bypass payload mapper"),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("system".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| {
                        content.contains("Follow the managed skill instruction before answering.")
                    })
                    .unwrap_or(false)
        }));
        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content.contains("[tool_loop_warning]\nwarning"))
                    .unwrap_or(false)
        }));
        assert!(
            tail.iter()
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .all(|content| !content.contains("[tool_result]\n[ok]"))
        );
    }

    #[test]
    fn tool_result_followup_tail_promotes_external_skill_from_semantic_envelope() {
        let tail = build_tool_result_followup_tail(
            "preface",
            r#"[ok] {"status":"ok","tool":"file.read","tool_call_id":"call-1","payload_semantics":"external_skill_context","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#,
            "summarize note.md",
            Some("warning"),
            |_, _| panic!("external skill payload should bypass payload mapper"),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("system".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| {
                        content.contains("Follow the managed skill instruction before answering.")
                    })
                    .unwrap_or(false)
        }));
        assert!(
            tail.iter()
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .all(|content| !content.contains("[tool_result]\n[ok]"))
        );
    }

    #[test]
    fn tool_result_followup_tail_uses_payload_mapper_and_keeps_truncation_hint() {
        let tail = build_tool_result_followup_tail(
            "preface",
            r#"[ok] {"payload_truncated":true}"#,
            "summarize note.md",
            Some("warning"),
            |_, _| "bounded-result".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_result]\nbounded-result")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_result_followup_tail_keeps_truncation_hint_when_payload_mapper_marks_result_truncated()
    {
        let tail = build_tool_result_followup_tail(
            "preface",
            r#"[ok] {"payload_truncated":false}"#,
            "summarize note.md",
            Some("warning"),
            |_, _| r#"[ok] {"payload_truncated":true}"#.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_failure_followup_tail_uses_payload_mapper_without_truncation_hint() {
        let tail = build_tool_failure_followup_tail(
            "preface",
            "tool_timeout ...(truncated 200 chars)",
            None,
            "summarize note.md",
            Some("warning"),
            |_, _| "bounded-failure".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_failure]\nbounded-failure")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(!user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_driven_followup_tail_dispatches_result_payload() {
        let payload = ToolDrivenFollowupPayload::ToolResult {
            text: r#"[ok] {"payload_truncated":true}"#.to_owned(),
        };
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            None,
            "summarize note.md",
            Some("warning"),
            |_, _| "bounded-result".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_result]\nbounded-result")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_driven_followup_tail_dispatches_failure_payload() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
            retryable: false,
        };
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            None,
            "summarize note.md",
            Some("warning"),
            |_, _| "bounded-failure".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_failure]\nbounded-failure")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(!user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_driven_followup_tail_preserves_request_summary_for_failure_payloads() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "payload.command contains path separators".to_owned(),
            retryable: false,
        };
        let tool_request_summary =
            r#"{"tool":"exec","request":{"command":"C:\\Windows\\System32\\RM.EXE"}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "summarize note.md",
            Some("warning"),
            |label, _| match label {
                "tool_request" => "bounded-request".to_owned(),
                _ => "bounded-failure".to_owned(),
            },
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_request]\nbounded-request")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains("Repair guidance for exec"));
        assert!(user_prompt.contains("retry with `rm.exe`"));
    }

    #[test]
    fn tool_driven_followup_tail_dispatches_discovery_recovery_payload() {
        let payload = ToolDrivenFollowupPayload::DiscoveryRecovery {
            reason: "tool_not_found: requested tool is not available If you still need a hidden capability, call tool.search.".to_owned(),
        };
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            None,
            "summarize note.md",
            Some("warning"),
            |_, _| "bounded-recovery".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_recovery]\nbounded-recovery")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(DISCOVERY_RECOVERY_FOLLOWUP_PROMPT));
        assert!(user_prompt.contains("Recovery reason:\nbounded-recovery"));
        assert!(!user_prompt.contains("tool_not_found"));
        assert!(
            user_prompt.contains("tool.invoke"),
            "discovery recovery prompt should explain the invoke step: {user_prompt}"
        );
        assert!(
            user_prompt.contains("lease"),
            "discovery recovery prompt should mention the lease requirement: {user_prompt}"
        );
        assert!(user_prompt.contains("Loop warning:\nwarning"));
    }

    #[test]
    fn tool_loop_guard_tail_uses_payload_mapper_and_builds_guard_prompt() {
        let latest_tool_context =
            ToolDrivenFollowupTextRef::new(ToolDrivenFollowupLabel::ToolResult, "tool output");
        let tail = build_tool_loop_guard_tail(
            "preface",
            "stop",
            "summarize note.md",
            Some(latest_tool_context),
            |_, _| "bounded-result".to_owned(),
        );

        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "preface")
                    .unwrap_or(false)
        }));
        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_result]\nbounded-result")
                    .unwrap_or(false)
        }));
        assert!(tail.iter().any(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content == "[tool_loop_guard]\nstop")
                    .unwrap_or(false)
        }));
        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(TOOL_LOOP_GUARD_PROMPT));
        assert!(user_prompt.contains("Loop guard reason:\nstop"));
        assert!(user_prompt.contains("Original request:\nsummarize note.md"));
    }

    #[test]
    fn tool_failure_followup_tail_strips_shell_arguments_from_repair_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
            retryable: false,
        };
        let tool_request_summary = r#"{"tool":"exec","request":{"command":"ls -la"}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "list the current directory",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for exec"));
        assert!(user_prompt.contains("The failed request used `ls -la`; retry with `ls`"));
    }

    #[test]
    fn tool_failure_followup_tail_strips_quoted_shell_arguments_from_repair_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
            retryable: false,
        };
        let tool_request_summary = r#"{"tool":"exec","request":{"command":"\"ls -la\" "}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "list the current directory",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for exec"));
        assert!(user_prompt.contains("The failed request used `\"ls -la\" `; retry with `ls`"));
    }

    #[test]
    fn tool_failure_followup_tail_renders_required_field_guidance_for_file_read() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason:
                "tool_preflight_denied: tool input needs repair: file.read payload.path is required (string)"
                    .to_owned(),
            retryable: false,
        };
        let tool_request_summary = r#"{"tool":"read","request":{}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "read the file",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for read"));
        assert!(user_prompt.contains("Add required field `payload.path` as a string."));
        assert!(user_prompt.contains(
            "Expected payload shape: path:string,offset?:integer,limit?:integer,max_bytes?:integer."
        ));
    }

    #[test]
    fn tool_failure_followup_tail_renders_hidden_agent_operation_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "hidden_agent_requires_operation: provide `operation` for archive, cancel, recover, or other multi-session control work".to_owned(),
            retryable: true,
        };
        let tool_request_summary = r#"{"tool":"agent","request":{"session_ids":["child-1"]}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "archive these sessions",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for agent"));
        assert!(user_prompt.contains("Add `operation`"));
        assert!(user_prompt.contains(r#"Current request preview: {"session_ids":["child-1"]}"#));
    }

    #[test]
    fn tool_failure_followup_tail_renders_hidden_skills_actionable_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason:
                "hidden_skills_requires_actionable_fields: provide actionable fields for grouped skills requests"
                    .to_owned(),
            retryable: true,
        };
        let tool_request_summary = r#"{"tool":"skills","request":{"query":"browser companion"}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "inspect the browser companion skill",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for skills"));
        assert!(user_prompt.contains("search, inspect, install, run, or list fields"));
        assert!(user_prompt.contains(r#"Current request preview: {"query":"browser companion"}"#));
    }

    #[test]
    fn tool_failure_followup_tail_renders_hidden_channel_operation_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "hidden_channel_requires_operation: provide `operation`, such as `messages.send`, `messages.reply`, `card.update`, or `feishu.whoami`".to_owned(),
            retryable: true,
        };
        let tool_request_summary = r#"{"tool":"channel","request":{"account_id":"default"}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "send a message",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for channel"));
        assert!(user_prompt.contains("messages.send"));
        assert!(user_prompt.contains(r#"Current request preview: {"account_id":"default"}"#));
    }

    #[test]
    fn tool_failure_followup_tail_uses_failure_reason_when_shell_summary_redacts_args_type() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.args must be array"
                .to_owned(),
            retryable: false,
        };
        let tool_request_summary = r#"{"tool":"exec","request":{"command":"echo"}}"#;
        let tail = build_tool_driven_followup_tail(
            "preface",
            &payload,
            Some(tool_request_summary),
            "run echo safely",
            None,
            |_, text| text.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for exec"));
        assert!(user_prompt.contains("Set `payload.args` to an array value."));
        assert!(user_prompt.contains(
            "Expected payload shape: command:string,args?:string[],timeout_ms?:integer,cwd?:string."
        ));
    }

    #[test]
    fn tool_loop_guard_tail_includes_truncation_hint_when_payload_mapper_truncates_result() {
        let latest_tool_context = ToolDrivenFollowupTextRef::new(
            ToolDrivenFollowupLabel::ToolResult,
            r#"[ok] {"payload_truncated":false}"#,
        );
        let tail = build_tool_loop_guard_tail(
            "preface",
            "stop",
            "summarize note.md",
            Some(latest_tool_context),
            |_, _| r#"[ok] {"payload_truncated":true}"#.to_owned(),
        );

        let user_prompt = tail
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(user_prompt.contains(TOOL_LOOP_GUARD_PROMPT));
        assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(user_prompt.contains("Loop guard reason:\nstop"));
    }

    #[test]
    fn tool_loop_guard_tail_skips_latest_tool_context_without_payload_mapping() {
        let tail = build_tool_loop_guard_tail("", "stop", "summarize note.md", None, |_, _| {
            panic!("missing latest tool context should bypass payload mapper")
        });

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0]["role"], "assistant");
        assert_eq!(tail[0]["content"], "[tool_loop_guard]\nstop");
        assert_eq!(tail[1]["role"], "user");
    }

    #[test]
    fn truncation_signal_detection_matches_structured_tool_result() {
        assert!(tool_result_contains_truncation_signal(
            r#"[ok] {"payload_truncated":true}"#
        ));
        assert!(tool_result_contains_truncation_signal(
            "payload ... (truncated 200 chars)"
        ));
        assert!(!tool_result_contains_truncation_signal(
            r#"[ok] {"payload_truncated":false}"#
        ));
    }

    #[test]
    fn truncation_signal_detection_ignores_payload_summary_lookalikes() {
        let deceptive_line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "payload_summary": r#"{"payload_truncated":true}"#,
                "payload_truncated": false
            })
        );
        assert!(!tool_result_contains_truncation_signal(
            deceptive_line.as_str()
        ));
    }

    #[test]
    fn followup_prompt_includes_truncation_hint_when_needed() {
        let prompt = build_tool_followup_user_prompt(
            "summarize this result",
            None,
            Some(r#"[ok] {"payload_truncated":true}"#),
            None,
            None,
        );
        assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(prompt.contains("Original request:\nsummarize this result"));
    }

    #[test]
    fn followup_prompt_includes_truncation_hint_when_rendered_payload_is_truncated() {
        let prompt = build_tool_followup_user_prompt(
            "summarize this result",
            None,
            Some(r#"[ok] {"payload_truncated":false}"#),
            Some(r#"[ok] {"payload_truncated":true}"#),
            None,
        );
        assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(prompt.contains("Original request:\nsummarize this result"));
    }

    #[test]
    fn followup_prompt_uses_discovery_guidance_for_discovery_shaped_results() {
        let payload_summary = json!({
            "query": "latest ai news",
            "results": [
                {
                    "tool_id": "web.search",
                    "lease": "lease-web-search"
                }
            ]
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-search",
                "payload_summary": payload_summary,
                "payload_chars": 512,
                "payload_truncated": false
            })
        );

        let prompt = build_tool_followup_user_prompt(
            "find the latest ai news and summarize it",
            None,
            Some(tool_result.as_str()),
            None,
            None,
        );

        assert!(prompt.contains(DISCOVERY_RESULT_FOLLOWUP_PROMPT));
        assert!(prompt.contains("Original request:\nfind the latest ai news and summarize it"));
    }

    #[test]
    fn followup_prompt_uses_generic_continuation_metadata_for_delegate_queue() {
        let payload_summary = json!({
            "child_session_id": "delegate:child-1",
            "mode": "async",
            "state": "queued",
            "continuation": {
                "state": "queued",
                "is_terminal": false,
                "recommended_tool": "session_wait",
                "recommended_payload": {
                    "session_id": "delegate:child-1",
                    "timeout_ms": 30000
                },
                "note": "The delegated child is still running in the background."
            }
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "delegate_async",
                "tool_call_id": "call-delegate-1",
                "payload_summary": payload_summary,
                "payload_chars": 256,
                "payload_truncated": false
            })
        );

        let prompt = build_tool_followup_user_prompt(
            "finish the delegated research and summarize the result",
            None,
            Some(tool_result.as_str()),
            None,
            None,
        );

        assert!(prompt.contains("Continuation guidance:"));
        assert!(prompt.contains("intermediate state `queued`"));
        assert!(prompt.contains("still running in the background"));
        assert!(prompt.contains("`session_wait`"));
        assert!(prompt.contains("{\"session_id\":\"delegate:child-1\",\"timeout_ms\":30000}"));
    }

    #[test]
    fn followup_prompt_uses_generic_continuation_metadata_for_waiting_task() {
        let payload_summary = json!({
            "wait_status": "waiting",
            "continuation": {
                "state": "waiting",
                "is_terminal": false,
                "note": "The runtime is still waiting on an approval or external completion gate."
            }
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "task_wait",
                "tool_call_id": "call-task-wait-1",
                "payload_summary": payload_summary,
                "payload_chars": 256,
                "payload_truncated": false
            })
        );

        let prompt = build_tool_followup_user_prompt(
            "wait until the task is complete and then summarize it",
            None,
            Some(tool_result.as_str()),
            None,
            None,
        );

        assert!(prompt.contains("Continuation guidance:"));
        assert!(prompt.contains("intermediate state `waiting`"));
        assert!(prompt.contains("approval or external completion gate"));
        assert!(prompt.contains("exact blocker"));
    }

    #[test]
    fn tool_result_payload_requests_runtime_followup_chain_for_nonterminal_continuation() {
        let payload = ToolDrivenFollowupPayload::ToolResult {
            text: format!(
                "[ok] {}",
                json!({
                    "status": "ok",
                    "tool": "session_wait",
                    "tool_call_id": "call-session-wait",
                    "payload_summary": json!({
                        "wait_status": "waiting",
                        "continuation": {
                            "state": "waiting",
                            "is_terminal": false,
                            "recommended_tool": "session_wait",
                            "recommended_payload": {
                                "session_id": "child-session",
                                "timeout_ms": 1000
                            }
                        }
                    })
                    .to_string(),
                    "payload_chars": 256,
                    "payload_truncated": false
                })
            ),
        };

        assert!(payload.requests_runtime_followup_chain());
    }

    #[test]
    fn tool_result_payload_does_not_request_runtime_followup_chain_for_terminal_continuation() {
        let payload = ToolDrivenFollowupPayload::ToolResult {
            text: format!(
                "[ok] {}",
                json!({
                    "status": "ok",
                    "tool": "session_wait",
                    "tool_call_id": "call-session-wait",
                    "payload_summary": json!({
                        "wait_status": "completed",
                        "continuation": {
                            "state": "completed",
                            "is_terminal": true
                        }
                    })
                    .to_string(),
                    "payload_chars": 256,
                    "payload_truncated": false
                })
            ),
        };

        assert!(!payload.requests_runtime_followup_chain());
    }

    #[test]
    fn reduce_followup_payload_for_model_preserves_shell_payload_metadata() {
        let payload = json!({
            "adapter": "core-tools",
            "tool_name": "shell.exec",
            "command": "cargo",
            "args": ["test", "--workspace"],
            "cwd": "/repo",
            "exit_code": 0,
            "stdout": format!("prefix {}", "x".repeat(512)),
            "stderr": "",
            "trace_id": "trace-123",
            "details": {
                "truncated": true,
                "handoff": {
                    "tool": "read",
                    "recommended_stream": "stdout",
                    "recommended_recipe": "last_page",
                    "recommended_payload": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200},
                    "recipes": {
                        "stdout": {
                            "recommended_recipe": "last_page",
                            "first_page": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 1, "limit": 200},
                            "last_page": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200},
                            "head": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 1, "limit": 200},
                            "tail": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200}
                        }
                    }
                }
            }
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "shell.exec",
                "tool_call_id": "call-shell",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        );

        let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
        let envelope: Value = serde_json::from_str(
            reduced
                .strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("reduced followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("shell payload summary should stay valid json");

        assert_eq!(envelope["tool"], "exec");
        assert_eq!(summary["adapter"], "core-tools");
        assert_eq!(summary["tool_name"], "shell.exec");
        assert_eq!(summary["trace_id"], "trace-123");
        assert_eq!(summary["command"], "cargo");
        assert_eq!(summary["exit_code"], 0);
        assert!(summary.get("stdout_preview").is_some());
        assert_eq!(summary["stdout_truncated"], true);
        assert_eq!(summary["details"]["handoff"]["tool"], json!("read"));
        assert_eq!(
            summary["details"]["handoff"]["recommended_stream"],
            json!("stdout")
        );
        assert_eq!(
            summary["details"]["handoff"]["recommended_recipe"],
            json!("last_page")
        );
        assert_eq!(
            summary["details"]["handoff"]["recipes"]["stdout"]["recommended_recipe"],
            json!("last_page")
        );
        assert_eq!(
            summary["details"]["handoff"]["recipes"]["stdout"]["last_page"]["offset"],
            json!(801)
        );
        assert_eq!(
            summary["details"]["handoff"]["recommended_payload"]["offset"],
            json!(801)
        );
    }

    #[test]
    fn reduce_followup_payload_for_model_counts_raw_shell_whitespace() {
        let payload = json!({
            "adapter": "core-tools",
            "tool_name": "shell.exec",
            "command": "printf",
            "args": ["%s", " "],
            "cwd": "/repo",
            "exit_code": 0,
            "stdout": " ".repeat(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS + 32),
            "stderr": "",
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "shell.exec",
                "tool_call_id": "call-shell",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        );

        let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
        let envelope: Value = serde_json::from_str(
            reduced
                .strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("reduced followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("shell payload summary should stay valid json");

        assert_eq!(summary["stdout_truncated"], true);
        assert_eq!(
            summary["stdout_chars"],
            json!(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS + 32)
        );
        assert_eq!(
            summary["stdout_preview"]
                .as_str()
                .expect("stdout preview should exist")
                .chars()
                .count(),
            SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS
        );
    }

    #[test]
    fn reduce_followup_payload_for_model_preserves_shell_tail_context() {
        let stdout = format!(
            "{}\n{}\n{}",
            "build log ".repeat(80),
            "intermediate output ".repeat(80),
            "final status: test suite failed on browser companion startup"
        );
        let payload = json!({
            "adapter": "core-tools",
            "tool_name": "shell.exec",
            "command": "cargo",
            "args": ["test", "--workspace"],
            "cwd": "/repo",
            "exit_code": 1,
            "stdout": stdout,
            "stderr": "",
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "shell.exec",
                "tool_call_id": "call-shell",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        );

        let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
        let envelope: Value = serde_json::from_str(
            reduced
                .strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("reduced followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("shell payload summary should stay valid json");
        let preview = summary["stdout_preview"]
            .as_str()
            .expect("stdout preview should exist");

        assert!(
            preview.contains("build log"),
            "preview should keep shell prefix"
        );
        assert!(
            preview.contains("final status: test suite failed on browser companion startup"),
            "preview should keep the final shell status"
        );
        assert!(
            preview.contains("[... omitted ...]"),
            "preview should signal when middle content is omitted"
        );
    }

    #[test]
    fn parse_external_skill_invoke_context_extracts_full_instructions_from_semantic_envelope() {
        let instructions = format!("prefix {}\nsuffix-marker", "x".repeat(256));
        let payload = json!({
            "skill_id": "demo-skill",
            "display_name": "Demo Skill",
            "instructions": instructions,
            "metadata": {
                "allowed_tools": ["shell.exec"],
                "blocked_tools": ["web.fetch"]
            }
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_semantics": "external_skill_context",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 512,
                "payload_truncated": false
            })
        );

        let parsed = parse_external_skill_invoke_context(line.as_str())
            .expect("invoke context should parse");
        assert_eq!(parsed.skill_id, "demo-skill");
        assert_eq!(parsed.display_name, "Demo Skill");
        assert!(parsed.instructions.contains("suffix-marker"));
        assert_eq!(parsed.allowed_tools, vec!["shell.exec"]);
        assert_eq!(parsed.blocked_tools, vec!["web.fetch"]);
    }

    #[test]
    fn parse_external_skill_invoke_context_requires_semantics_or_legacy_tool_name() {
        let payload = json!({
            "skill_id": "demo-skill",
            "display_name": "Demo Skill",
            "instructions": "Follow the managed skill instruction before answering.",
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-1",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 512,
                "payload_truncated": false
            })
        );

        assert!(
            parse_external_skill_invoke_context(line.as_str()).is_none(),
            "skill-shaped payloads should not activate managed skill context without semantics or the legacy tool name"
        );
    }

    #[test]
    fn parse_external_skill_invoke_context_rejects_truncated_payload() {
        let payload = json!({
            "skill_id": "demo-skill",
            "display_name": "Demo Skill",
            "instructions": "Follow the managed skill instruction before answering.",
        });
        let line = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "external_skills.invoke",
                "tool_call_id": "call-1",
                "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
                "payload_chars": 512,
                "payload_truncated": true
            })
        );

        assert!(
            parse_external_skill_invoke_context(line.as_str()).is_none(),
            "truncated external skill payload should not activate managed skill context"
        );
    }

    #[test]
    fn reduce_followup_payload_for_model_compacts_tool_search_summary() {
        let payload_summary = json!({
            "adapter": "core-tools",
            "tool_name": "tool.search",
            "query": "read repo file",
            "exact_tool_id": "file.read",
            "returned": 1,
            "diagnostics": {
                "reason": "exact_tool_id_not_visible",
                "requested_tool_id": "file.read"
            },
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "tags": ["core", "file", "read"],
                    "why": ["summary matches query"],
                    "lease": "lease-file"
                }
            ]
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "tool.search",
                "tool_call_id": "call-search",
                "payload_summary": payload_summary,
                "payload_chars": 512,
                "payload_truncated": false
            })
        );

        let reduced = reduce_followup_payload_for_model("tool_result", tool_result.as_str());
        let envelope: Value = serde_json::from_str(
            reduced
                .strip_prefix("[ok] ")
                .expect("tool result should keep status prefix"),
        )
        .expect("reduced envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("reduced payload summary should stay valid json");
        let first = summary["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("reduced payload should keep the first result");

        assert_eq!(summary["query"], "read repo file");
        assert_eq!(summary["exact_tool_id"], "file.read");
        assert_eq!(
            summary["diagnostics"]["reason"],
            "exact_tool_id_not_visible"
        );
        assert!(summary.get("adapter").is_none());
        assert!(summary.get("tool_name").is_none());
        assert_eq!(summary["returned"], 1);
        assert_eq!(first["tool_id"], "file.read");
        assert_eq!(first["lease"], "lease-file");
        assert!(first.get("tags").is_none());
        assert!(first.get("why").is_none());
    }

    #[test]
    fn reduce_followup_payload_for_model_preserves_empty_required_arrays() {
        let payload_summary = json!({
            "query": "install a skill",
            "results": [
                {
                    "tool_id": "external_skills.install",
                    "summary": "Install a bundled skill or a local skill path.",
                    "argument_hint": "bundled_skill_id?:string,path?:string",
                    "required_fields": [],
                    "required_field_groups": [],
                    "lease": "lease-install"
                }
            ]
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "tool.search",
                "tool_call_id": "call-search",
                "payload_summary": payload_summary,
                "payload_chars": 512,
                "payload_truncated": false
            })
        );

        let reduced = reduce_followup_payload_for_model("tool_result", tool_result.as_str());
        let envelope: Value = serde_json::from_str(
            reduced
                .strip_prefix("[ok] ")
                .expect("tool result should keep status prefix"),
        )
        .expect("reduced envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("reduced payload summary should stay valid json");
        let first = summary["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("reduced payload should keep the first result");

        assert_eq!(first["required_fields"], json!([]));
        assert_eq!(first["required_field_groups"], json!([]));
    }

    #[test]
    fn reduce_followup_payload_for_model_borrows_unmodified_tool_results() {
        let tool_result = r#"[ok] {"status":"ok","tool":"tool.search","tool_call_id":"call-search","payload_summary":"{\"query\":\"status\"}","payload_chars":32,"payload_truncated":true}"#;

        let reduced = reduce_followup_payload_for_model("tool_result", tool_result);

        assert_eq!(reduced.as_ref(), tool_result);
        assert_eq!(reduced.as_ptr(), tool_result.as_ptr());
    }

    #[test]
    fn summarize_failed_provider_lane_tool_request_preserves_multi_intent_context_without_trace() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "Cargo.toml"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "shell.exec".to_owned(),
                    args_json: json!({"command": "ls /root"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-a".to_owned(),
                    turn_id: "turn-a".to_owned(),
                    tool_call_id: "call-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        };

        let request_summary = summarize_failed_provider_lane_tool_request(&turn, None)
            .expect("multi-intent failures should retain a request summary");
        let request_summary_json: Value =
            serde_json::from_str(&request_summary).expect("request summary should be valid json");
        let request_entries = request_summary_json
            .as_array()
            .expect("multi-intent request summary should be an array");

        assert_eq!(request_entries.len(), 2);
        assert_eq!(request_entries[0]["tool"], "read");
        assert_eq!(request_entries[1]["tool"], "exec");
        assert_eq!(request_entries[1]["request"]["command"], "ls");
        assert_eq!(request_entries[1]["request"]["args_redacted"], 1);
    }

    #[test]
    fn summarize_single_tool_followup_request_resolves_grouped_hidden_invoke_to_precise_operation()
    {
        let intent = ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: json!({
                "tool_id": "agent",
                "lease": "lease-agent",
                "arguments": {
                    "operation": "delegate-background",
                    "task": "summarize the repo"
                }
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-agent".to_owned(),
        };

        let request_summary = summarize_single_tool_followup_request(&intent)
            .expect("grouped hidden invoke should retain a request summary");
        let request_summary_json: Value =
            serde_json::from_str(&request_summary).expect("request summary should be valid json");

        assert_eq!(request_summary_json["tool"], "delegate_async");
        assert_eq!(
            request_summary_json["request"]["task"],
            "summarize the repo"
        );
        assert!(request_summary_json["request"].get("operation").is_none());
    }

    #[test]
    fn strip_think_tags_removes_think_content() {
        let input = "<think>Let me think about this...\nThe user wants to know the weather.\nI should check the forecast.</think>The weather today is sunny.";
        let expected = "The weather today is sunny.";
        assert_eq!(strip_think_tags(input), expected);
    }

    #[test]
    fn strip_think_tags_handles_empty_tags() {
        let input = "Hello <think></think>world";
        assert_eq!(strip_think_tags(input), "Hello world");
    }

    #[test]
    fn strip_think_tags_handles_multiple_tags() {
        let input = "<think>First thought</think>Middle<think>Second thought</think>End";
        assert_eq!(strip_think_tags(input), "MiddleEnd");
    }

    #[test]
    fn strip_think_tags_handles_nested_content() {
        let input = "<think>Think content with <tag> inside</think>Real response";
        assert_eq!(strip_think_tags(input), "Real response");
    }

    #[test]
    fn strip_think_tags_handles_nested_think_tags() {
        let input = "<think>outer<think>inner</think>visible</think>done";
        assert_eq!(strip_think_tags(input), "done");
    }

    #[test]
    fn strip_think_tags_case_insensitive() {
        let input = "<ThInK>think content</tHiNk>Result";
        assert_eq!(strip_think_tags(input), "Result");
    }

    #[test]
    fn strip_think_tags_drops_unterminated_opening_tag() {
        let input = "Answer<think>internal reasoning";
        assert_eq!(strip_think_tags(input), "Answer");
    }

    #[test]
    fn strip_think_tags_drops_stray_closing_tag() {
        let input = "Answer</think>";
        assert_eq!(strip_think_tags(input), "Answer");
    }
}
