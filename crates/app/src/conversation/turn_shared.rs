use super::persistence::format_provider_error_reply;
use super::turn_engine::TurnResult;
use serde_json::Value;

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";
pub const EXTERNAL_SKILL_FOLLOWUP_PROMPT: &str = "A managed external skill has been loaded into runtime context. Follow its instructions while answering the original user request. Do not restate the skill verbatim unless the user explicitly asks for it.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSkillInvokeContext {
    pub skill_id: String,
    pub display_name: String,
    pub instructions: String,
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
        TurnResult::NeedsApproval(failure) => {
            let inline = format!("[tool_approval_required] {}", failure.reason);
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
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
) -> String {
    let mut sections = vec![TOOL_FOLLOWUP_PROMPT.to_owned()];
    if let Some(reason) = loop_warning_reason {
        sections.push(format!(
            "Loop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient."
        ));
    }
    if tool_result_text
        .map(tool_result_contains_truncation_signal)
        .unwrap_or(false)
    {
        sections.push(TOOL_TRUNCATION_HINT_PROMPT.to_owned());
    }
    sections.push(format!("Original request:\n{user_input}"));
    sections.join("\n\n")
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

pub fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
mod tests {
    use super::*;
    use crate::conversation::turn_engine::{TurnFailure, TurnResult};
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
        );
        assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
        assert!(prompt.contains("Original request:\nsummarize this result"));
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
}
