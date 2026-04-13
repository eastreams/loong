use serde_json::{Value, json};

use super::{AcpSessionState, AcpTurnStopReason};

#[derive(Debug, Clone, Default)]
pub(super) struct AcpxIdentifiers {
    pub(super) acpx_record_id: Option<String>,
    pub(super) backend_session_id: Option<String>,
    pub(super) agent_session_id: Option<String>,
}

pub(super) fn parse_json_lines(stdout: &str) -> Vec<Value> {
    stdout.lines().filter_map(parse_json_line).collect()
}

pub(super) fn parse_json_line(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

pub(super) fn is_done_event(event: &Value) -> bool {
    raw_string(event, "type").as_deref() == Some("done")
}

pub(super) fn extract_identifiers(events: &[Value]) -> AcpxIdentifiers {
    let mut identifiers = AcpxIdentifiers::default();
    for event in events {
        if identifiers.acpx_record_id.is_none() {
            identifiers.acpx_record_id =
                value_string(event, "acpxRecordId").or_else(|| raw_string(event, "record_id"));
        }
        if identifiers.backend_session_id.is_none() {
            identifiers.backend_session_id = value_string(event, "acpxSessionId")
                .or_else(|| value_string(event, "backendSessionId"))
                .or_else(|| value_string(event, "session_id"));
        }
        if identifiers.agent_session_id.is_none() {
            identifiers.agent_session_id = value_string(event, "agentSessionId")
                .or_else(|| value_string(event, "agent_session_id"));
        }
    }
    identifiers
}

pub(super) fn collect_output_text(events: &[Value]) -> String {
    let mut text = String::new();
    for event in events {
        let event_type = raw_string(event, "type");
        if let Some(chunk) = extract_output_chunk(event) {
            let needs_spacing = event_type.as_deref() == Some("text")
                && !text.is_empty()
                && !text.ends_with(char::is_whitespace)
                && !chunk.starts_with(char::is_whitespace);
            if needs_spacing {
                text.push(' ');
            }
            text.push_str(chunk.as_str());
        }
    }
    text
}

pub(super) fn extract_output_chunk(event: &Value) -> Option<String> {
    let event_type = raw_string(event, "type")?;
    match event_type.as_str() {
        "text" => raw_string(event, "content"),
        "output" | "assistant" | "message" | "text_delta" | "agent_message_chunk" => {
            nested_text(event)
        }
        _ => None,
    }
}

pub(super) fn collect_usage_update(events: &[Value]) -> Option<Value> {
    events.iter().rev().find_map(|event| {
        let direct_usage = raw_string(event, "type").as_deref() == Some("usage_update");
        let tagged_usage = raw_string(event, "sessionUpdate").as_deref() == Some("usage_update");
        let nested_usage = event
            .get("params")
            .and_then(|params| params.get("update"))
            .and_then(|payload| payload.get("sessionUpdate"))
            .and_then(Value::as_str)
            == Some("usage_update");
        if !(direct_usage || tagged_usage || nested_usage) {
            return None;
        }

        let payload = event
            .get("params")
            .and_then(|params| params.get("update"))
            .unwrap_or(event);
        let used = payload.get("used").and_then(Value::as_u64);
        let size = payload.get("size").and_then(Value::as_u64);
        if used.is_none() && size.is_none() {
            return None;
        }

        let mut usage = serde_json::Map::new();
        if let Some(used) = used {
            usage.insert("used".to_owned(), json!(used));
        }
        if let Some(size) = size {
            usage.insert("size".to_owned(), json!(size));
        }
        Some(Value::Object(usage))
    })
}

pub(super) fn collect_stop_reason(events: &[Value]) -> Option<AcpTurnStopReason> {
    for event in events.iter().rev() {
        let Some(event_type) = raw_string(event, "type") else {
            continue;
        };
        if event_type == "done" {
            let reason = raw_string(event, "stopReason")
                .or_else(|| raw_string(event, "stop_reason"))
                .or_else(|| raw_string(event, "reason"))
                .unwrap_or_default();
            return match reason.as_str() {
                "cancelled" | "canceled" => Some(AcpTurnStopReason::Cancelled),
                _ => Some(AcpTurnStopReason::Completed),
            };
        }
    }
    None
}

pub(super) fn nested_text(value: &Value) -> Option<String> {
    value_string(value, "text")
        .or_else(|| {
            value
                .get("delta")
                .and_then(|delta| value_string(delta, "text"))
        })
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| value_string(message, "content"))
        })
        .or_else(|| {
            value
                .get("content")
                .and_then(Value::as_array)
                .and_then(|parts| {
                    let collected = parts
                        .iter()
                        .filter_map(|part| {
                            value_string(part, "text")
                                .or_else(|| value_string(part, "content"))
                                .or_else(|| {
                                    part.get("text")
                                        .and_then(|text| value_string(text, "value"))
                                })
                        })
                        .collect::<String>();
                    (!collected.is_empty()).then_some(collected)
                })
        })
}

pub(super) fn raw_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(normalized_non_empty)
}

pub(super) fn value_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|inner| match inner {
        Value::String(text) => normalized_non_empty(text),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
    })
}

pub(super) fn normalized_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(super) fn event_error_message(events: &[Value], ignore_no_session: bool) -> Option<String> {
    for event in events.iter().rev() {
        let Some(event_type) = raw_string(event, "type") else {
            continue;
        };
        if !matches!(event_type.as_str(), "error" | "failed") {
            continue;
        }
        let code = event_code(std::slice::from_ref(event));
        let should_ignore_no_session = code
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("no_session"));
        if ignore_no_session && should_ignore_no_session {
            continue;
        }
        if let Some(message) = raw_string(event, "message")
            .or_else(|| raw_string(event, "error"))
            .or_else(|| nested_text(event))
        {
            return Some(message);
        }
    }
    None
}

pub(super) fn event_code(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|event| raw_string(event, "code").or_else(|| raw_string(event, "error_code")))
}

pub(super) fn map_status_state(raw: Option<&str>) -> AcpSessionState {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "initializing" => AcpSessionState::Initializing,
        "busy" | "running" => AcpSessionState::Busy,
        "cancelling" | "canceling" => AcpSessionState::Cancelling,
        "error" | "failed" => AcpSessionState::Error,
        "closed" | "stopped" => AcpSessionState::Closed,
        _ => AcpSessionState::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_stop_reason_ignores_typeless_tail_events() {
        let events = vec![
            json!({"type": "done", "stopReason": "cancelled"}),
            json!({"usage": {"used": 12}}),
        ];

        let stop_reason = collect_stop_reason(&events);

        assert_eq!(stop_reason, Some(AcpTurnStopReason::Cancelled));
    }

    #[test]
    fn event_error_message_ignores_typeless_tail_events() {
        let events = vec![
            json!({"type": "error", "message": "session exploded"}),
            json!({"usage": {"used": 12}}),
        ];

        let error_message = event_error_message(&events, false);

        assert_eq!(error_message.as_deref(), Some("session exploded"));
    }

    #[test]
    fn event_error_message_ignores_no_session_case_insensitively() {
        let events = vec![json!({
            "type": "failed",
            "code": "NO_SESSION",
            "message": "session missing"
        })];

        let error_message = event_error_message(&events, true);

        assert_eq!(error_message, None);
    }
}
