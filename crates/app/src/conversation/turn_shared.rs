use super::super::config::LoongClawConfig;
use super::ProviderErrorMode;
use super::persistence::format_provider_error_reply;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::tool_input_contract::render_tool_input_repair_guidance;
use super::tool_result_compaction::compact_tool_search_payload_summary_str;
use super::turn_engine::{
    ApprovalRequirement, ApprovalRequirementKind, ProviderTurn, ToolBatchExecutionIntentStatus,
    ToolBatchExecutionTrace, ToolIntent, ToolResultEnvelope, ToolResultPayloadSemantics,
    TurnFailure, TurnResult,
};
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

use crate::CliResult;

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
pub const DISCOVERY_RESULT_FOLLOWUP_PROMPT: &str = "The tool result above is a discovery result, not the final evidence. Choose the best matching discovered tool, reuse its lease when invoking it, continue with the next tool call needed to satisfy the original user request, and only answer directly if the discovery results already contain the final user-facing information.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";
pub const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "An external skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";
pub const DISCOVERY_RECOVERY_FOLLOWUP_PROMPT: &str = "The previous tool call could not be executed as requested. If you still need a hidden or discoverable capability, call tool.search with a short natural-language description of the missing capability. Otherwise, provide the best possible answer with the currently available evidence.";
pub const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";

const FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS: usize = 384;
const SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS: usize = 384;
const SHELL_FOLLOWUP_STDIO_OMISSION_MARKER: &str = "\n[... omitted ...]\n";
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
    let stripped_text = strip_think_tags(text);
    let trimmed_text = stripped_text.trim();
    trimmed_text.to_owned()
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
    ToolFailure { reason: String },
    DiscoveryRecovery { reason: String },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultLine {
    status_marker: String,
    envelope: ToolResultEnvelope,
}

impl ToolResultLine {
    pub fn new(status_marker: impl Into<String>, envelope: ToolResultEnvelope) -> Self {
        Self {
            status_marker: status_marker.into(),
            envelope,
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        let (status_prefix, payload) = trimmed.split_once(' ')?;
        let status_marker = status_prefix.strip_prefix('[')?.strip_suffix(']')?.trim();
        if status_marker.is_empty() {
            return None;
        }
        let envelope = serde_json::from_str::<ToolResultEnvelope>(payload).ok()?;
        Some(Self::new(status_marker, envelope))
    }

    pub fn render(&self) -> Option<String> {
        let payload = serde_json::to_string(&self.envelope).ok()?;
        Some(format!("[{}] {payload}", self.status_marker))
    }

    pub fn tool_name(&self) -> &str {
        self.envelope.tool.as_str()
    }

    pub fn payload_truncated(&self) -> bool {
        self.envelope.payload_truncated
    }

    pub fn set_payload_truncated(&mut self, truncated: bool) {
        self.envelope.payload_truncated = truncated;
    }

    pub fn payload_summary_str(&self) -> &str {
        self.envelope.payload_summary.as_str()
    }

    pub fn payload_summary_json(&self) -> Option<Value> {
        serde_json::from_str(self.envelope.payload_summary.as_str()).ok()
    }

    pub fn replace_payload_summary_str(&mut self, payload_summary: String) {
        self.envelope.payload_summary = payload_summary;
    }

    pub fn envelope(&self) -> &ToolResultEnvelope {
        &self.envelope
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
            Self::ToolFailure { reason } => ToolDrivenFollowupTextRef::new(label, reason.as_str()),
            Self::DiscoveryRecovery { reason } => {
                ToolDrivenFollowupTextRef::new(label, reason.as_str())
            }
        }
    }
}

pub fn turn_failure_supports_discovery_recovery(failure: &TurnFailure) -> bool {
    let is_tool_not_found = failure.code == "tool_not_found";
    let has_recovery_hint = failure.supports_discovery_recovery;
    is_tool_not_found && has_recovery_hint
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
    let tool_name = effective_followup_tool_name(intent);
    let request = effective_followup_request(intent);
    let request = sanitize_followup_request_summary(tool_name.as_str(), request);
    serde_json::json!({
        "tool": tool_name,
        "request": request,
    })
}

fn sanitize_followup_request_summary(tool_name: &str, request: Value) -> Value {
    if tool_name != crate::tools::SHELL_EXEC_TOOL_NAME {
        return request;
    }

    sanitize_shell_followup_request_summary(request)
}

fn sanitize_shell_followup_request_summary(request: Value) -> Value {
    let Value::Object(request_object) = request else {
        return request;
    };

    let command = request_object
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let timeout_ms = request_object.get("timeout_ms").cloned();
    let args_redacted = request_object
        .get("args")
        .and_then(Value::as_array)
        .map(Vec::len)
        .filter(|count| *count > 0);

    let mut sanitized_request = serde_json::Map::new();

    if let Some(command) = command {
        sanitized_request.insert("command".to_owned(), Value::String(command));
    }

    if let Some(timeout_ms) = timeout_ms {
        sanitized_request.insert("timeout_ms".to_owned(), timeout_ms);
    }

    if let Some(args_redacted) = args_redacted {
        let args_redacted = serde_json::Number::from(args_redacted);
        sanitized_request.insert("args_redacted".to_owned(), Value::Number(args_redacted));
    }

    Value::Object(sanitized_request)
}

pub(crate) fn effective_followup_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    if canonical_tool_name != "tool.invoke" {
        return canonical_tool_name.to_owned();
    }

    intent
        .args_json
        .get("tool_id")
        .and_then(Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .unwrap_or(canonical_tool_name)
        .to_owned()
}

pub(crate) fn effective_followup_request(intent: &ToolIntent) -> Value {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    if canonical_tool_name != "tool.invoke" {
        return crate::tools::normalize_shell_payload_for_request(
            canonical_tool_name,
            intent.args_json.clone(),
        );
    }

    let invoked_tool_name = intent
        .args_json
        .get("tool_id")
        .and_then(Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .unwrap_or(canonical_tool_name);
    let request_payload = intent
        .args_json
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| intent.args_json.clone());

    crate::tools::normalize_shell_payload_for_request(invoked_tool_name, request_payload)
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
                .map(|tool_name| format!("LoongClaw wants to call {tool_name}"))
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
        tool_name: requirement.tool_name.clone(),
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
    if let Some(extra_context) = extra_context {
        sections.push(extra_context.to_owned());
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
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

pub fn reduce_followup_payload_for_model<'a>(label: &str, text: &'a str) -> Cow<'a, str> {
    if label != "tool_result" {
        return Cow::Borrowed(text);
    }

    reduce_tool_result_text_for_model(text)
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed(text))
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

fn reduce_tool_result_line_for_model(line: &str) -> String {
    let Some(mut tool_result_line) = ToolResultLine::parse(line) else {
        return line.to_owned();
    };
    let tool = tool_result_line.tool_name();
    let payload_truncated = tool_result_line.payload_truncated();
    let payload_summary = tool_result_line.payload_summary_str();
    if payload_summary.is_empty() {
        return line.to_owned();
    }
    let reduction = match tool {
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
        _ if !payload_truncated => {
            compact_tool_search_payload_summary_str(payload_summary).map(|summary| (summary, false))
        }
        _ => None,
    };
    let Some((reduced_summary, mark_truncated)) = reduction else {
        return line.to_owned();
    };
    if mark_truncated {
        tool_result_line.set_payload_truncated(true);
    }
    tool_result_line.replace_payload_summary_str(reduced_summary);
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

pub fn build_tool_result_followup_tail<F>(
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
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
        "content": build_tool_followup_user_prompt(
            user_input,
            loop_warning_reason,
            Some(tool_result_text),
            Some(bounded_result.as_str()),
            None,
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

pub fn build_tool_failure_followup_tail_with_request_summary<F>(
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
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
            repair_guidance.as_deref(),
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
    match payload {
        ToolDrivenFollowupPayload::ToolResult { text } => build_tool_result_followup_tail(
            assistant_preface,
            text.as_str(),
            user_input,
            loop_warning_reason,
            payload_mapper,
        ),
        ToolDrivenFollowupPayload::ToolFailure { reason } => {
            build_tool_failure_followup_tail_with_request_summary(
                assistant_preface,
                reason.as_str(),
                user_input,
                loop_warning_reason,
                tool_request_summary,
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
    let tool_name = request_summary_json.get("tool").and_then(Value::as_str)?;
    let request_summary_request = request_summary_json.get("request");
    let reason_mentions_repairable_shape = tool_failure_reason.contains("tool input needs repair")
        || tool_failure_reason.contains("payload must be an object")
        || tool_failure_reason.contains("payload.");

    if !reason_mentions_repairable_shape {
        return None;
    }

    let shell_guidance = render_shell_failure_repair_guidance(
        tool_name,
        request_summary_request,
        tool_failure_reason,
    );

    if shell_guidance.is_some() {
        return shell_guidance;
    }

    render_tool_input_repair_guidance(tool_name, request_summary_request)
}

fn render_shell_failure_repair_guidance(
    tool_name: &str,
    request_summary_request: Option<&Value>,
    tool_failure_reason: &str,
) -> Option<String> {
    if tool_name != "shell.exec" {
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
    let guidance = format!(
        "Repair guidance for shell.exec:\nUse a bare lowercase executable name in `payload.command`.\nThe failed request used `{command}`; retry with `{bare_command}`."
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

pub async fn request_completion_with_raw_fallback<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    messages: &[Value],
    binding: ConversationRuntimeBinding<'_>,
    raw_reply: &str,
) -> String {
    match runtime.request_completion(config, messages, binding).await {
        Ok(final_reply) => {
            let sanitized_reply = sanitize_reply_text(final_reply.as_str());
            if sanitized_reply.is_empty() {
                sanitize_reply_text(raw_reply)
            } else {
                sanitized_reply
            }
        }
        Err(_) => sanitize_reply_text(raw_reply),
    }
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
    Some(ExternalSkillInvokeContext {
        skill_id,
        display_name,
        instructions,
    })
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
        };
        let message_context = payload.message_context();

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolFailure);
        assert_eq!(
            message_context.label(),
            ToolDrivenFollowupLabel::ToolFailure
        );
        assert_eq!(message_context.label().as_str(), "tool_failure");
        assert_eq!(message_context.text(), "tool failed");
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
    fn turn_failure_supports_discovery_recovery_requires_structured_metadata() {
        let recovery_failure = TurnFailure::policy_denied_with_discovery_recovery(
            "tool_not_found",
            "tool_not_found: requested tool is not available If you need a non-core capability, call tool.search with a short natural-language description of the task.",
        );
        let plain_failure = TurnFailure::policy_denied(
            "tool_not_found",
            "tool_not_found: requested tool is not available",
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
        };
        let tool_request_summary =
            r#"{"tool":"shell.exec","request":{"command":"C:\\Windows\\System32\\RM.EXE"}}"#;
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
        assert!(user_prompt.contains("Repair guidance for shell.exec"));
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
        };
        let tool_request_summary = r#"{"tool":"shell.exec","request":{"command":"ls -la"}}"#;
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

        assert!(user_prompt.contains("Repair guidance for shell.exec"));
        assert!(user_prompt.contains("The failed request used `ls -la`; retry with `ls`"));
    }

    #[test]
    fn tool_failure_followup_tail_strips_quoted_shell_arguments_from_repair_guidance() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
        };
        let tool_request_summary = r#"{"tool":"shell.exec","request":{"command":"\"ls -la\" "}}"#;
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

        assert!(user_prompt.contains("Repair guidance for shell.exec"));
        assert!(user_prompt.contains("The failed request used `\"ls -la\" `; retry with `ls`"));
    }

    #[test]
    fn tool_failure_followup_tail_renders_required_field_guidance_for_file_read() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason:
                "tool_preflight_denied: tool input needs repair: file.read payload.path is required (string)"
                    .to_owned(),
        };
        let tool_request_summary = r#"{"tool":"file.read","request":{}}"#;
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

        assert!(user_prompt.contains("Repair guidance for file.read"));
        assert!(user_prompt.contains("Add required field `payload.path` as a string."));
        assert!(user_prompt.contains("Expected payload shape: path:string,max_bytes?:integer."));
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

        assert_eq!(summary["adapter"], "core-tools");
        assert_eq!(summary["tool_name"], "shell.exec");
        assert_eq!(summary["trace_id"], "trace-123");
        assert_eq!(summary["command"], "cargo");
        assert_eq!(summary["exit_code"], 0);
        assert!(summary.get("stdout_preview").is_some());
        assert_eq!(summary["stdout_truncated"], true);
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
        let tool_result = r#"[ok] {"status":"ok","tool":"shell.exec","tool_call_id":"call-shell","payload_summary":"{\"stdout\":\"hello\"}","payload_chars":32,"payload_truncated":false}"#;

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
        assert_eq!(request_entries[0]["tool"], "file.read");
        assert_eq!(request_entries[1]["tool"], "shell.exec");
        assert_eq!(request_entries[1]["request"]["command"], "ls");
        assert_eq!(request_entries[1]["request"]["args_redacted"], 1);
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
