//! Request body construction for the OpenAI-compatible endpoint.

use contracts::{
    provider::Request,
    tool::ToolSpec,
    transcript::{Role, TranscriptItem},
};
use serde_json::{Value, json};

pub(super) fn build_body(req: &Request, model: &str) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(transcript_item_to_message)
        .collect();

    let tools: Vec<Value> = req.tools.iter().map(tool_spec_to_function).collect();

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    body
}

fn transcript_item_to_message(item: &TranscriptItem) -> Value {
    match item {
        TranscriptItem::Message { role, text } => json!({
            "role": role_name(role),
            "content": text,
        }),
        TranscriptItem::ToolCall {
            call_id,
            name,
            arguments,
        } => json!({
            "role": "assistant",
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments,
                },
            }],
        }),
        TranscriptItem::ToolResult { call_id, output } => json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": output,
        }),
    }
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn tool_spec_to_function(tool: &ToolSpec) -> Value {
    let parameters = serde_json::to_value(&tool.input_schema).unwrap_or_else(|_| json!({}));
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": parameters,
        },
    })
}
