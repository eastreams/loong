use super::persistence::format_provider_error_reply;
use super::turn_engine::TurnResult;
use serde_json::Value;

pub const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
pub const TOOL_TRUNCATION_HINT_PROMPT: &str = "One or more tool results were truncated for context safety. If exact missing details are needed, explicitly state the truncation and request a narrower rerun.";

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

pub fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
}
