use serde_json::Value;

use super::tool_result::{
    followup_prompt_needs_truncation_hint, proactive_followup_continuation_context,
};

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to continue satisfying the original user request. Prefer the next bounded tool call or completion step over narrating intermediate status. If the result is only a path listing, metadata summary, or other partial evidence and the user still needs file or page contents, issue the next read or fetch call instead of asking the user to approve more inspection. Only stop to answer in natural language when the request is actually complete, blocked on a real approval or input gate, or the available evidence is already sufficient. If you are finishing now, output only the final answer text and omit planning prefaces, raw JSON, payload wrappers, `to=` pseudo commands, and status markers unless the user explicitly asks for raw output.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";
pub const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "A skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";
pub const TOOL_FAILURE_FOLLOWUP_PROMPT: &str = "The previous tool call could not be executed as requested. Retry with a valid direct tool call, a corrected payload, or answer with the best available evidence if the missing tool action is no longer necessary.";
pub const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";

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
    let mut sections = vec![TOOL_FOLLOWUP_PROMPT.to_owned()];
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

pub(crate) fn combine_followup_extra_context(parts: &[Option<&str>]) -> Option<String> {
    let joined = parts
        .iter()
        .flatten()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    (!joined.is_empty()).then(|| joined.join("\n\n"))
}

pub fn build_discovery_recovery_followup_user_prompt(
    user_input: &str,
    loop_warning_reason: Option<&str>,
    recovery_reason: &str,
) -> String {
    let mut sections = vec![TOOL_FAILURE_FOLLOWUP_PROMPT.to_owned()];
    sections.push(format!("Recovery reason:\n{recovery_reason}"));
    sections.push(
        "Prefer a valid direct tool call or a refreshed visible tool request. Do not fall back to hidden discovery-first wrapper syntax."
            .to_owned(),
    );
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating identical unavailable tool calls. Refresh the visible tool request or change strategy."
        ));
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
}

pub fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn append_followup_preface(messages: &mut Vec<Value>, assistant_preface: &str) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": preface,
        }));
    }
}

pub(crate) fn append_followup_warning(
    messages: &mut Vec<Value>,
    loop_warning_reason: Option<&str>,
) {
    if let Some(reason) = loop_warning_reason {
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
}
