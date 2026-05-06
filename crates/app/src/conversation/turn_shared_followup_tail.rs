use serde_json::Value;

use super::super::tool_input_contract::{
    render_tool_input_repair_guidance, render_tool_input_repair_guidance_from_reason,
    repair_guidance_visible_tool_name,
};
use super::tool_result::followup_prompt_needs_truncation_hint;
use super::{
    EXTERNAL_SKILL_FOLLOWUP_PROMPT, TOOL_LOOP_GUARD_PROMPT, TOOL_TRUNCATION_HINT_PROMPT,
    ToolDrivenFollowupLabel, ToolDrivenFollowupPayload, ToolDrivenFollowupTextRef,
    append_followup_preface, append_followup_warning, combine_followup_extra_context,
    parse_skill_context,
};

pub fn build_skill_system_message(skill_context: &super::SkillContext) -> String {
    format!(
        "Skill `{}` ({}) is now active for this task. Treat the following `SKILL.md` content as trusted runtime guidance until superseded.\n\n{}",
        skill_context.skill_id, skill_context.display_name, skill_context.instructions
    )
}

pub fn build_skill_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    skill_context: &super::SkillContext,
) -> String {
    let mut sections = vec![
        EXTERNAL_SKILL_FOLLOWUP_PROMPT.to_owned(),
        format!(
            "Loaded skill:\n- id: {}\n- name: {}",
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
    if let Some(skill_context) = parse_skill_context(tool_result_text) {
        messages.push(serde_json::json!({
            "role": "system",
            "content": build_skill_system_message(&skill_context),
        }));
        append_followup_warning(&mut messages, loop_warning_reason);
        messages.push(serde_json::json!({
            "role": "user",
            "content": build_skill_followup_user_prompt(
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
        "content": super::build_tool_followup_user_prompt_with_context(
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
        "content": super::build_tool_followup_user_prompt_with_context(
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
        "content": super::build_discovery_recovery_followup_user_prompt(
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
    let summary_tool_name = request_summary_json
        .get("tool")
        .or_else(|| request_summary_json.get("name"))
        .and_then(Value::as_str)?;
    let repair_tool_name = repair_guidance_tool_name(summary_tool_name, tool_failure_reason);
    let request_summary_request = request_summary_json
        .get("request")
        .or_else(|| request_summary_json.get("arguments"));
    let direct_routing_guidance = render_direct_routing_failure_repair_guidance(
        repair_tool_name.as_str(),
        request_summary_request,
        tool_failure_reason,
    );

    if direct_routing_guidance.is_some() {
        return direct_routing_guidance;
    }

    let byte_budget_guidance = render_byte_budget_failure_repair_guidance(
        repair_tool_name.as_str(),
        request_summary_request,
        tool_failure_reason,
    );
    if byte_budget_guidance.is_some() {
        return byte_budget_guidance;
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

fn render_byte_budget_failure_repair_guidance(
    tool_name: &str,
    request_summary_request: Option<&Value>,
    tool_failure_reason: &str,
) -> Option<String> {
    if !tool_failure_reason.contains("max_bytes limit") {
        return None;
    }

    let request_object = request_summary_request?.as_object()?;
    let mut retry_request = request_object.clone();
    let current_max_bytes = retry_request
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(match tool_name {
            "read" => 65_536,
            "browser" => 32_768,
            "web" => 32_768,
            _ => 32_768,
        });
    let suggested_max_bytes = current_max_bytes.clamp(1_024, 32_768);
    let suggested_max_bytes = if suggested_max_bytes < current_max_bytes {
        suggested_max_bytes
    } else {
        current_max_bytes.max(2_048) / 2
    };
    retry_request.insert(
        "max_bytes".to_owned(),
        serde_json::json!(suggested_max_bytes),
    );
    let retry_call = serde_json::json!({
        "name": tool_name,
        "arguments": Value::Object(retry_request),
    });
    let retry_call = serde_json::to_string_pretty(&retry_call).ok()?;

    let mut lines = vec![format!("Repair guidance for {tool_name}:")];
    match tool_name {
        "web" => {
            lines.push(
                "Retry the same web request with a smaller `max_bytes`. If you only need search hits, keep the request in query mode instead of escalating to a full page fetch.".to_owned(),
            );
        }
        "browser" => {
            lines.push(
                "Retry the browser extraction with a smaller `max_bytes` or a more focused extraction target.".to_owned(),
            );
        }
        _ => {
            lines.push(
                "Retry the same request with a smaller `max_bytes` or a narrower read window."
                    .to_owned(),
            );
        }
    }
    lines.push("Suggested retry:".to_owned());
    lines.push(retry_call);
    Some(lines.join("\n"))
}

fn render_direct_routing_failure_repair_guidance(
    tool_name: &str,
    request_summary_request: Option<&Value>,
    tool_failure_reason: &str,
) -> Option<String> {
    let normalized_reason = tool_failure_reason
        .strip_prefix("tool execution failed: ")
        .or_else(|| {
            tool_failure_reason.strip_prefix("tool_preflight_denied: tool input needs repair: ")
        })
        .or_else(|| tool_failure_reason.strip_prefix("tool input needs repair: "))
        .unwrap_or(tool_failure_reason);

    let guidance = if normalized_reason.starts_with("hidden_agent_requires_operation:") {
        "Add `operation` for runtime-control requests such as session archive, cancel, recover, or approval workflows.".to_owned()
    } else if normalized_reason.starts_with("hidden_agent_requires_actionable_fields:") {
        "Add the concrete session / approval / delegate / provider / config fields needed for the request, or set `operation` when the legacy control wrapper request is ambiguous.".to_owned()
    } else if normalized_reason.starts_with("hidden_skills_requires_actionable_fields:") {
        "Add search, inspect, install, run, or list fields for the legacy skills-management request, or provide `operation` to make the request explicit.".to_owned()
    } else if normalized_reason.starts_with("hidden_channel_requires_operation:") {
        "Add `operation` for the channel request, for example `messages.send`, `messages.reply`, `card.update`, or `feishu.whoami`.".to_owned()
    } else if normalized_reason.starts_with("direct_read_requires_one_of:") {
        "Provide exactly one read mode: `path` for a file, `query` for content search, or `pattern` for glob-style path matching.".to_owned()
    } else if normalized_reason.starts_with("direct_read_ambiguous:") {
        "Use only one read mode at a time. Do not mix `path`, `query`, and `pattern` in the same direct read request.".to_owned()
    } else if normalized_reason.starts_with("direct_write_requires_path:") {
        "Add `path` to tell the direct write surface which file to replace.".to_owned()
    } else if normalized_reason.starts_with("direct_write_requires_content:") {
        "Add `content` with the full replacement text for the direct write request.".to_owned()
    } else if normalized_reason.starts_with("direct_edit_requires_edits:") {
        "Add `edits` with one or more exact edit blocks, and include `path`.".to_owned()
    } else if normalized_reason.starts_with("direct_edit_requires_path:") {
        "Add `path` so the direct edit surface knows which file to patch.".to_owned()
    } else if normalized_reason.starts_with("direct_bash_requires_command:") {
        "Add `command` with the bash command string to execute.".to_owned()
    } else if normalized_reason.starts_with("direct_web_requires_query_or_url:") {
        "Provide `query` for search mode, or `url` for fetch/request mode.".to_owned()
    } else if normalized_reason.starts_with("direct_web_ambiguous:") {
        "Do not mix `query` with `url` or low-level request fields in the same direct web call."
            .to_owned()
    } else if normalized_reason.starts_with("direct_browser_page_click_requires_session_id:") {
        "Add `session_id` before issuing page click actions through the direct browser surface."
            .to_owned()
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
    if crate::tools::user_visible_tool_name(tool_name) != "bash" {
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
