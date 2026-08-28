//! Request body construction for the OpenAI-compatible endpoint.

use contracts::{
    provider::Request,
    tool::ToolSpec,
    transcript::{Role, TranscriptItem},
};
use serde_json::{Value, json};

pub(super) fn build_body(req: &Request, model: &str) -> Value {
    let messages = build_messages(&req.messages);

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

fn build_messages(items: &[TranscriptItem]) -> Vec<Value> {
    let mut messages = Vec::new();
    let mut assistant: Option<Value> = None;
    let mut tool_calls: Vec<Value> = Vec::new();

    for item in items {
        match item {
            TranscriptItem::Message {
                role: Role::Assistant,
                text,
                reasoning_content,
            } => {
                flush_assistant(&mut messages, &mut assistant, &mut tool_calls);
                let mut message = json!({
                    "role": "assistant",
                    "content": text,
                });
                if let Some(reasoning) = reasoning_content {
                    message["reasoning_content"] = json!(reasoning);
                }
                assistant = Some(message);
            }
            TranscriptItem::ToolCall {
                call_id,
                name,
                arguments,
                reasoning_content,
            } => {
                if assistant.is_none() {
                    assistant = Some(json!({ "role": "assistant" }));
                }
                tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    },
                }));
                if let Some(reasoning) = reasoning_content
                    && let Some(message) = assistant.as_mut()
                    && message.get("reasoning_content").is_none()
                {
                    message["reasoning_content"] = json!(reasoning);
                }
            }
            TranscriptItem::Message {
                role,
                text,
                reasoning_content,
            } => {
                flush_assistant(&mut messages, &mut assistant, &mut tool_calls);
                let mut message = json!({
                    "role": role_name(role),
                    "content": text,
                });
                if let Some(reasoning) = reasoning_content {
                    message["reasoning_content"] = json!(reasoning);
                }
                messages.push(message);
            }
            TranscriptItem::ToolResult { call_id, output } => {
                flush_assistant(&mut messages, &mut assistant, &mut tool_calls);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
        }
    }

    flush_assistant(&mut messages, &mut assistant, &mut tool_calls);
    messages
}

fn flush_assistant(
    messages: &mut Vec<Value>,
    assistant: &mut Option<Value>,
    tool_calls: &mut Vec<Value>,
) {
    let Some(mut message) = assistant.take() else {
        tool_calls.clear();
        return;
    };

    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(std::mem::take(tool_calls));
    }
    messages.push(message);
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

#[cfg(test)]
mod tests {
    use contracts::{
        provider::Request,
        transcript::{Role, TranscriptItem},
    };

    use super::build_body;

    #[test]
    fn message_emits_reasoning_content_when_present() {
        let req = Request {
            messages: vec![TranscriptItem::Message {
                role: Role::Assistant,
                text: "answer".to_string(),
                reasoning_content: Some("think".to_string()),
            }],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        assert_eq!(body["messages"][0]["reasoning_content"], "think");
    }

    #[test]
    fn tool_call_emits_reasoning_content_when_present() {
        let req = Request {
            messages: vec![TranscriptItem::ToolCall {
                call_id: "call_1".to_string(),
                name: "echo".to_string(),
                arguments: "{}".to_string(),
                reasoning_content: Some("think".to_string()),
            }],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        assert_eq!(body["messages"][0]["reasoning_content"], "think");
    }

    #[test]
    fn assistant_text_and_tool_calls_share_one_message() {
        let req = Request {
            messages: vec![
                TranscriptItem::Message {
                    role: Role::Assistant,
                    text: "let me check".to_string(),
                    reasoning_content: Some("think".to_string()),
                },
                TranscriptItem::ToolCall {
                    call_id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                    reasoning_content: None,
                },
                TranscriptItem::ToolCall {
                    call_id: "call_2".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                    reasoning_content: None,
                },
            ],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "let me check");
        assert_eq!(messages[0]["reasoning_content"], "think");
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn assistant_text_and_tool_calls_merge_without_reasoning() {
        let req = Request {
            messages: vec![
                TranscriptItem::Message {
                    role: Role::Assistant,
                    text: "let me check".to_string(),
                    reasoning_content: None,
                },
                TranscriptItem::ToolCall {
                    call_id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: "{}".to_string(),
                    reasoning_content: None,
                },
            ],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "let me check");
        assert!(messages[0].get("reasoning_content").is_none());
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn tool_call_without_reasoning_omits_reasoning_content() {
        let req = Request {
            messages: vec![TranscriptItem::ToolCall {
                call_id: "call_1".to_string(),
                name: "echo".to_string(),
                arguments: "{}".to_string(),
                reasoning_content: None,
            }],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].get("reasoning_content").is_none());
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn reasoning_content_is_omitted_when_absent() {
        let req = Request {
            messages: vec![TranscriptItem::Message {
                role: Role::User,
                text: "hi".to_string(),
                reasoning_content: None,
            }],
            tools: Vec::new(),
        };

        let body = build_body(&req, "test-model");
        assert!(body["messages"][0].get("reasoning_content").is_none());
    }
}
