use serde_json::{Map, Value, json};
use std::io::{self, BufRead};

fn build_extension_payload(operation: &str, payload: &Map<String, Value>) -> Value {
    match operation {
        "extension/event" => {
            let handled_event = payload
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            json!({
                "ok": true,
                "handled_event": handled_event,
            })
        }
        "extension/command" => {
            let command_name = payload
                .get("command_name")
                .and_then(Value::as_str)
                .unwrap_or("extension");
            json!({
                "text": format!("{command_name} command stub"),
            })
        }
        "extension/resource" => json!({
            "commands": [],
            "tools": [],
        }),
        other => json!({
            "error": format!("unsupported method: {other}"),
        }),
    }
}

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<Value>(trimmed) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let payload = request
            .get("payload")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let response_payload = if method == "tools/call" {
            let operation = payload
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let extension_payload = payload
                .get("payload")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            build_extension_payload(operation, &extension_payload)
        } else {
            json!({
                "error": format!("unsupported transport method: {method}"),
            })
        };

        println!(
            "{}",
            json!({
                "method": method,
                "id": id,
                "payload": response_payload,
            })
        );
    }
}
