use std::collections::BTreeSet;

use loong_contracts::{Capability, ToolCoreRequest};
use serde_json::Value;

pub(crate) const TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD: &str = "_granted_capabilities";
pub(crate) const TOOL_LEASE_TOKEN_ID_FIELD: &str = "_lease_token_id";
pub(crate) const TOOL_LEASE_SESSION_ID_FIELD: &str = "_lease_session_id";
pub(crate) const TOOL_LEASE_TURN_ID_FIELD: &str = "_lease_turn_id";

pub(crate) fn normalize_shell_payload_for_request(tool_name: &str, payload: Value) -> Value {
    match super::canonical_tool_name(tool_name) {
        "shell.exec" => normalize_shell_payload_object(payload),
        "tool.invoke" => normalize_shell_invoke_payload(payload),
        _ => payload,
    }
}

pub(crate) fn normalize_shell_request_for_execution(
    mut request: ToolCoreRequest,
) -> ToolCoreRequest {
    request.payload =
        normalize_shell_payload_for_request(request.tool_name.as_str(), request.payload);
    request
}

pub fn summarize_tool_request_for_display(tool_name: &str, payload: Value) -> Value {
    let canonical_tool_name = super::canonical_tool_name(tool_name);
    let normalized_payload = normalize_shell_payload_for_request(canonical_tool_name, payload);

    if canonical_tool_name != super::SHELL_EXEC_TOOL_NAME {
        return normalized_payload;
    }

    summarize_shell_request_for_display(normalized_payload)
}

pub(crate) fn prepare_kernel_tool_request(
    mut request: ToolCoreRequest,
    granted_capabilities: &BTreeSet<Capability>,
    token_id: Option<&str>,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> ToolCoreRequest {
    request = normalize_shell_request_for_execution(request);
    let canonical_tool_name = super::canonical_tool_name(request.tool_name.as_str());
    if !matches!(canonical_tool_name, "tool.search" | "tool.invoke") {
        return request;
    }

    if let Value::Object(payload) = &mut request.payload {
        if canonical_tool_name == "tool.search" {
            let granted_capabilities_json =
                serde_json::to_value(granted_capabilities.iter().copied().collect::<Vec<_>>());
            let granted_capabilities_json =
                granted_capabilities_json.unwrap_or_else(|_| Value::Array(Vec::new()));
            payload.insert(
                TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD.to_owned(),
                granted_capabilities_json,
            );
        }
        inject_tool_lease_binding(payload, token_id, session_id, turn_id);
    }

    request
}

fn summarize_shell_request_for_display(request: Value) -> Value {
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

    let mut summarized_request = serde_json::Map::new();

    if let Some(command) = command {
        summarized_request.insert("command".to_owned(), Value::String(command));
    }

    if let Some(timeout_ms) = timeout_ms {
        summarized_request.insert("timeout_ms".to_owned(), timeout_ms);
    }

    if let Some(args_redacted) = args_redacted {
        let args_redacted = serde_json::Number::from(args_redacted);
        summarized_request.insert("args_redacted".to_owned(), Value::Number(args_redacted));
    }

    Value::Object(summarized_request)
}

fn normalize_shell_invoke_payload(payload: Value) -> Value {
    let mut outer = match payload {
        Value::Object(outer) => outer,
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_)
        | other @ Value::Array(_) => return other,
    };

    let tool_id = outer
        .get("tool_id")
        .and_then(Value::as_str)
        .map(super::canonical_tool_name);
    let Some(tool_id) = tool_id else {
        return Value::Object(outer);
    };
    if tool_id != "shell.exec" {
        return Value::Object(outer);
    }

    let arguments = outer
        .remove("arguments")
        .map(normalize_shell_payload_object)
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    outer.insert("arguments".to_owned(), arguments);
    Value::Object(outer)
}

fn normalize_shell_payload_object(payload: Value) -> Value {
    let mut object = match payload {
        Value::Object(object) => object,
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_)
        | other @ Value::Array(_) => return other,
    };

    let args_missing = match object.get("args") {
        None => true,
        Some(Value::Array(values)) => values.is_empty(),
        Some(_) => false,
    };
    if !args_missing {
        return Value::Object(object);
    }

    let command = object.get("command").and_then(Value::as_str);
    let Some(command) = command else {
        return Value::Object(object);
    };
    let split_command = split_shell_command_if_safe(command);
    let Some((normalized_command, normalized_args)) = split_command else {
        return Value::Object(object);
    };

    object.insert("command".to_owned(), Value::String(normalized_command));
    if normalized_args.is_empty() {
        object.remove("args");
    } else {
        let args_json = normalized_args
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        object.insert("args".to_owned(), Value::Array(args_json));
    }
    Value::Object(object)
}

fn split_shell_command_if_safe(command: &str) -> Option<(String, Vec<String>)> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    let contains_newline = trimmed.chars().any(|ch| matches!(ch, '\n' | '\r'));
    if contains_newline {
        return None;
    }

    let contains_whitespace = trimmed.contains(char::is_whitespace);
    if !contains_whitespace {
        return None;
    }

    let contains_risky_shell_syntax = trimmed.chars().any(|ch| {
        matches!(
            ch,
            '\'' | '"' | '\\' | '`' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '$'
        )
    });
    if contains_risky_shell_syntax {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let command = parts.next()?.to_owned();
    let args = parts.map(str::to_owned).collect::<Vec<_>>();
    if args.is_empty() {
        return None;
    }

    Some((command, args))
}

pub(crate) fn inject_tool_lease_binding(
    payload: &mut serde_json::Map<String, Value>,
    token_id: Option<&str>,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) {
    if let Some(token_id) = token_id {
        payload.insert(
            TOOL_LEASE_TOKEN_ID_FIELD.to_owned(),
            Value::String(token_id.to_owned()),
        );
    }
    if let Some(session_id) = session_id {
        payload.insert(
            TOOL_LEASE_SESSION_ID_FIELD.to_owned(),
            Value::String(session_id.to_owned()),
        );
    }
    if let Some(turn_id) = turn_id {
        payload.insert(
            TOOL_LEASE_TURN_ID_FIELD.to_owned(),
            Value::String(turn_id.to_owned()),
        );
    }
}
