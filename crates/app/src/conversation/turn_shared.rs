use super::super::config::LoongClawConfig;
use super::ProviderErrorMode;
use super::persistence::format_provider_error_reply;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_engine::{ApprovalRequirement, ApprovalRequirementKind, ProviderTurn, TurnResult};
use serde::Serialize;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::CliResult;

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";
pub const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "A managed external skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";
pub const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";

const FILE_READ_FOLLOWUP_CONTENT_PREVIEW_CHARS: usize = 384;
const SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS: usize = 384;
const SHELL_FOLLOWUP_STDIO_OMISSION_MARKER: &str = "\n[... omitted ...]\n";

pub fn next_conversation_turn_id() -> String {
    static NEXT_CONVERSATION_TURN_SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = NEXT_CONVERSATION_TURN_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("turn-{nanos:x}-{seq:x}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDrivenFollowupPayload {
    ToolResult { text: String },
    ToolFailure { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDrivenFollowupKind {
    ToolResult,
    ToolFailure,
}

impl ToolDrivenFollowupPayload {
    pub fn kind(&self) -> ToolDrivenFollowupKind {
        match self {
            Self::ToolResult { .. } => ToolDrivenFollowupKind::ToolResult,
            Self::ToolFailure { .. } => ToolDrivenFollowupKind::ToolFailure,
        }
    }

    pub fn message_context(&self) -> (&'static str, &str) {
        match self {
            Self::ToolResult { text } => ("tool_result", text.as_str()),
            Self::ToolFailure { reason } => ("tool_failure", reason.as_str()),
        }
    }
}

pub fn tool_driven_followup_payload(
    had_tool_intents: bool,
    turn_result: &TurnResult,
) -> Option<ToolDrivenFollowupPayload> {
    if !had_tool_intents {
        return None;
    }

    match turn_result {
        TurnResult::FinalText(text) => {
            Some(ToolDrivenFollowupPayload::ToolResult { text: text.clone() })
        }
        TurnResult::NeedsApproval(_) => None,
        TurnResult::ToolDenied(failure) | TurnResult::ToolError(failure) => {
            Some(ToolDrivenFollowupPayload::ToolFailure {
                reason: failure.reason.clone(),
            })
        }
        TurnResult::ProviderError(_) => None,
    }
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
            TurnResult::FinalText(text) => Some(join_non_empty_lines(&[
                self.assistant_preface,
                text.as_str(),
            ])),
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
        if raw_tool_output_requested {
            ToolDrivenReplyBaseDecision::FinalizeDirect { reply: raw_reply }
        } else {
            ToolDrivenReplyBaseDecision::RequireFollowup { raw_reply, payload }
        }
    }
}

pub fn user_requested_raw_tool_output(user_input: &str) -> bool {
    let normalized = user_input.to_ascii_lowercase();
    [
        "raw",
        "json",
        "payload",
        "verbatim",
        "exact output",
        "full output",
        "tool output",
        "[ok]",
    ]
    .iter()
    .any(|signal| normalized.contains(signal))
}

pub fn compose_assistant_reply(
    assistant_preface: &str,
    had_tool_intents: bool,
    turn_result: TurnResult,
) -> String {
    match turn_result {
        TurnResult::FinalText(text) => {
            if had_tool_intents {
                join_non_empty_lines(&[assistant_preface, text.as_str()])
            } else {
                text
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
    let mut lines = Vec::new();
    let marker = match requirement.kind {
        ApprovalRequirementKind::GovernedTool => "[tool_approval_required]",
        ApprovalRequirementKind::KernelContextRequired => "[approval_required]",
    };
    lines.push(marker.to_owned());
    if let Some(tool_name) = requirement.tool_name.as_deref() {
        lines.push(format!("tool: {tool_name}"));
    }
    if let Some(request_id) = requirement.approval_request_id.as_deref() {
        lines.push(format!("request_id: {request_id}"));
    }
    if let Some(approval_key) = requirement.approval_key.as_deref() {
        lines.push(format!("approval_key: {approval_key}"));
    }
    lines.push(format!("rule_id: {}", requirement.rule_id));
    lines.push(format!("reason: {}", requirement.reason));
    if requirement.kind == ApprovalRequirementKind::GovernedTool {
        lines.push("allowed_decisions: approve_once, approve_always, deny".to_owned());
    }
    let body = lines.join("\n");
    join_non_empty_lines(&[assistant_preface, body.as_str()])
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
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
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
        return false;
    }
    let envelope = match serde_json::from_str::<Value>(candidate) {
        Ok(value) => value,
        Err(_) => return false,
    };
    envelope
        .get("payload_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn build_tool_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    tool_result_text: Option<&str>,
    rendered_tool_result_text: Option<&str>,
) -> String {
    let mut sections = vec![TOOL_FOLLOWUP_PROMPT.to_owned()];
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient."
        ));
    }
    if followup_prompt_needs_truncation_hint(tool_result_text, rendered_tool_result_text) {
        sections.push(TOOL_TRUNCATION_HINT_PROMPT.to_owned());
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

fn reduce_tool_result_line_for_model(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return line.to_owned();
    }
    let Some((status_prefix, payload)) = trimmed.split_once(' ') else {
        return line.to_owned();
    };
    if !(status_prefix.starts_with('[') && status_prefix.ends_with(']')) {
        return line.to_owned();
    }
    let Ok(mut envelope) = serde_json::from_str::<Value>(payload) else {
        return line.to_owned();
    };
    let Some(tool) = envelope.get("tool").and_then(Value::as_str) else {
        return line.to_owned();
    };

    let payload_truncated = envelope
        .get("payload_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(payload_summary) = envelope.get("payload_summary").and_then(Value::as_str) else {
        return line.to_owned();
    };

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
        "tool.search" if !payload_truncated => {
            compact_tool_search_payload_summary_str(payload_summary).map(|summary| (summary, false))
        }
        _ => None,
    };
    let Some((reduced_summary, mark_truncated)) = reduction else {
        return line.to_owned();
    };

    let Some(envelope_object) = envelope.as_object_mut() else {
        return line.to_owned();
    };
    envelope_object.insert("payload_summary".to_owned(), Value::String(reduced_summary));
    if mark_truncated {
        envelope_object.insert("payload_truncated".to_owned(), Value::Bool(true));
    }
    let Ok(encoded) = serde_json::to_string(&envelope) else {
        return line.to_owned();
    };
    format!("{status_prefix} {encoded}")
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
        "Managed external skill `{}` ({}) is now active for this task. Treat the following `SKILL.md` content as trusted runtime guidance until superseded.\n\n{}",
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
            "Loaded managed external skill:\n- id: {}\n- name: {}",
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

fn compact_tool_search_payload_summary_str(payload_summary: &str) -> Option<String> {
    let payload_json = serde_json::from_str::<Value>(payload_summary).ok()?;
    let compacted_summary = compact_tool_search_payload_summary(&payload_json)?;
    let compacted_summary_str = serde_json::to_string(&compacted_summary).ok()?;
    (compacted_summary_str.len() < payload_summary.len()).then_some(compacted_summary_str)
}

fn compact_tool_search_payload_summary(payload: &Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let results = payload_object.get("results")?.as_array()?;

    let mut compacted = Map::new();
    if let Some(query) = payload_object.get("query") {
        compacted.insert("query".to_owned(), query.clone());
    }
    compacted.insert(
        "results".to_owned(),
        Value::Array(
            results
                .iter()
                .map(compact_tool_search_payload_result)
                .collect(),
        ),
    );

    Some(Value::Object(compacted))
}

fn compact_tool_search_payload_result(result: &Value) -> Value {
    let Some(result_object) = result.as_object() else {
        return result.clone();
    };

    let mut compacted = Map::new();
    clone_field_if_present(result_object, &mut compacted, "tool_id");
    clone_field_if_present(result_object, &mut compacted, "summary");
    clone_field_if_present(result_object, &mut compacted, "argument_hint");
    clone_array_field_if_present(result_object, &mut compacted, "required_fields");
    clone_array_field_if_present(result_object, &mut compacted, "required_field_groups");
    clone_field_if_present(result_object, &mut compacted, "lease");
    Value::Object(compacted)
}

fn clone_field_if_present(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_owned(), value.clone());
    }
}

fn clone_array_field_if_present(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    key: &str,
) {
    let Some(value) = source.get(key) else {
        return;
    };
    if value.as_array().is_some() {
        target.insert(key.to_owned(), value.clone());
    }
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

    let bounded_result = payload_mapper("tool_result", tool_result_text);
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": format!("[tool_result]\n{bounded_result}"),
    }));
    append_followup_warning(&mut messages, loop_warning_reason);
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_tool_followup_user_prompt(
            user_input,
            loop_warning_reason,
            Some(tool_result_text),
            Some(bounded_result.as_str()),
        ),
    }));
    messages
}

pub fn build_tool_failure_followup_tail<F>(
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    let mut messages = Vec::new();
    append_followup_preface(&mut messages, assistant_preface);
    let bounded_failure = payload_mapper("tool_failure", tool_failure_reason);
    messages.push(serde_json::json!({
        "role": "assistant",
        "content": format!("[tool_failure]\n{bounded_failure}"),
    }));
    append_followup_warning(&mut messages, loop_warning_reason);
    messages.push(serde_json::json!({
        "role": "user",
        "content": build_tool_followup_user_prompt(user_input, loop_warning_reason, None, None),
    }));
    messages
}

pub fn build_tool_driven_followup_tail<F>(
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    user_input: &str,
    loop_warning_reason: Option<&str>,
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
        ToolDrivenFollowupPayload::ToolFailure { reason } => build_tool_failure_followup_tail(
            assistant_preface,
            reason.as_str(),
            user_input,
            loop_warning_reason,
            payload_mapper,
        ),
    }
}

pub fn build_tool_loop_guard_tail<F>(
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<(&str, &str)>,
    mut payload_mapper: F,
) -> Vec<Value>
where
    F: FnMut(&str, &str) -> String,
{
    let mut messages = Vec::new();
    let mut original_tool_result_text = None;
    let mut rendered_tool_result_text = None;
    append_followup_preface(&mut messages, assistant_preface);
    if let Some((label, text)) = latest_tool_context {
        let bounded = payload_mapper(label, text);
        if label == "tool_result" {
            original_tool_result_text = Some(text);
            rendered_tool_result_text = Some(bounded.clone());
        }
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": format!("[{label}]\n{bounded}"),
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
            let trimmed = final_reply.trim();
            if trimmed.is_empty() {
                raw_reply.to_owned()
            } else {
                trimmed.to_owned()
            }
        }
        Err(_) => raw_reply.to_owned(),
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
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let payload = trimmed.strip_prefix("[ok] ")?;
    let envelope: Value = serde_json::from_str(payload).ok()?;
    if envelope.get("tool")?.as_str()? != "external_skills.invoke" {
        return None;
    }
    if envelope
        .get("payload_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let payload_summary = envelope.get("payload_summary")?.as_str()?;
    let payload_json: Value = serde_json::from_str(payload_summary).ok()?;
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
        .expect("assistant tool_result followup message should exist");
    let line = assistant_tool_result
        .lines()
        .nth(1)
        .expect("assistant tool_result should keep payload line");
    let envelope: Value = serde_json::from_str(
        line.strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("followup envelope should stay valid json");
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
        assert!(reply.contains("approve_once"));
        assert!(reply.contains("approve_always"));
        assert!(reply.contains("deny"));
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

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolResult);
        assert_eq!(payload.message_context(), ("tool_result", "tool output"));
    }

    #[test]
    fn tool_driven_followup_payload_reports_failure_kind_and_context() {
        let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool failed".to_owned(),
        };

        assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolFailure);
        assert_eq!(payload.message_context(), ("tool_failure", "tool failed"));
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
                "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\napproval_key: tool:delegate_async\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: approve_once, approve_always, deny"
            )
        );
        assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\napproval_key: tool:delegate_async\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: approve_once, approve_always, deny".to_owned(),
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
    fn tool_loop_guard_tail_uses_payload_mapper_and_builds_guard_prompt() {
        let tail = build_tool_loop_guard_tail(
            "preface",
            "stop",
            "summarize note.md",
            Some(("tool_result", "tool output")),
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
    fn tool_loop_guard_tail_includes_truncation_hint_when_payload_mapper_truncates_result() {
        let tail = build_tool_loop_guard_tail(
            "preface",
            "stop",
            "summarize note.md",
            Some(("tool_result", r#"[ok] {"payload_truncated":false}"#)),
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
        );
        assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(prompt.contains("Original request:\nsummarize this result"));
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
    fn parse_external_skill_invoke_context_extracts_full_instructions() {
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
                "tool": "external_skills.invoke",
                "tool_call_id": "call-1",
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
            "returned": 1,
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
        assert!(summary.get("adapter").is_none());
        assert!(summary.get("tool_name").is_none());
        assert!(summary.get("returned").is_none());
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
}
