use loong_contracts::ToolCoreRequest;
use serde_json::Value;

pub(crate) fn normalize_shell_payload_for_request(tool_name: &str, payload: Value) -> Value {
    match super::canonical_tool_name(tool_name) {
        "shell.exec" => normalize_shell_payload_object(payload),
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

pub(crate) fn summarize_shell_request_for_display(request: Value) -> Value {
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

#[cfg(test)]
mod tests {
    use super::super::tool_request_prep::summarize_tool_request_for_display;
    use serde_json::json;

    #[test]
    fn summarize_tool_request_for_display_redacts_bash_exec_arguments() {
        let summary = summarize_tool_request_for_display(
            "bash.exec",
            json!({
                "command": "git",
                "args": ["status", "--short"],
                "timeout_ms": 3000
            }),
        );

        assert_eq!(summary["command"], "git");
        assert_eq!(summary["timeout_ms"], 3000);
        assert_eq!(summary["args_redacted"], 2);
        assert!(summary.get("args").is_none());
    }
}
