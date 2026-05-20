use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::conversation::turn_engine::{ProviderTurn, ToolIntent};
use crate::tools;

mod json_tool_call;
mod model_catalog;
mod inline_function;

use inline_function::{
    attach_inline_function_parse_telemetry, extract_inline_function_call_turn,
    InlineFunctionParseResult,
};

pub fn extract_provider_turn(body: &Value) -> Option<ProviderTurn> {
    extract_provider_turn_with_scope(body, None, None)
}

pub fn extract_provider_turn_with_scope(
    body: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> Option<ProviderTurn> {
    extract_provider_turn_with_scope_and_messages(body, session_id, turn_id, &[])
}

pub fn extract_provider_turn_with_scope_and_messages(
    body: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    messages: &[Value],
) -> Option<ProviderTurn> {
    let bridge_context = provider_tool_bridge_context_from_messages(messages);

    if let Some(turn) = extract_responses_provider_turn(body, session_id, turn_id, &bridge_context)
    {
        return Some(turn);
    }

    if let Some(message) = openai_message(body) {
        let mut assistant_text = message_content(message).unwrap_or_default();
        let mut raw_meta = message.clone();
        if let Some(usage) = body.get("usage")
            && let Some(raw_meta_object) = raw_meta.as_object_mut()
        {
            raw_meta_object.insert("usage".to_owned(), usage.clone());
        }
        let mut tool_intents =
            extract_openai_tool_intents(message, session_id, turn_id, &bridge_context);

        if tool_intents.is_empty() {
            let extraction = extract_openai_text_tool_turn(
                assistant_text.as_str(),
                &mut raw_meta,
                session_id,
                turn_id,
                &bridge_context,
            );
            assistant_text = extraction.assistant_text;
            tool_intents = extraction.tool_intents;
        }

        return Some(ProviderTurn {
            assistant_text,
            tool_intents,
            raw_meta,
        });
    }

    if let Some(message) = bedrock_message(body) {
        return Some(ProviderTurn {
            assistant_text: message_content(message).unwrap_or_default(),
            tool_intents: extract_bedrock_tool_intents(
                message,
                session_id,
                turn_id,
                &bridge_context,
            ),
            raw_meta: normalize_bedrock_message(message),
        });
    }

    if let Some(message) = google_message(body) {
        let assistant_text = google_message_content(message).unwrap_or_default();
        let tool_intents =
            extract_google_tool_intents(message, session_id, turn_id, &bridge_context);
        if assistant_text.is_empty() && tool_intents.is_empty() {
            return None;
        }

        return Some(ProviderTurn {
            assistant_text,
            tool_intents,
            raw_meta: body.clone(),
        });
    }

    let assistant_text = extract_body_content_text(body).unwrap_or_default();
    let tool_intents = extract_anthropic_tool_intents(body, session_id, turn_id, &bridge_context);
    if assistant_text.is_empty() && tool_intents.is_empty() {
        return None;
    }

    Some(ProviderTurn {
        assistant_text,
        tool_intents,
        raw_meta: body.clone(),
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProviderToolBridgeContext;

struct OpenAiTextToolTurnExtraction {
    assistant_text: String,
    tool_intents: Vec<ToolIntent>,
}

fn provider_tool_bridge_context_from_messages(messages: &[Value]) -> ProviderToolBridgeContext {
    let _ = messages;
    ProviderToolBridgeContext
}

fn extract_openai_text_tool_turn(
    assistant_text: &str,
    raw_meta: &mut Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> OpenAiTextToolTurnExtraction {
    let mut cleaned_text = assistant_text.to_owned();
    let mut tool_intents = Vec::new();

    if let Some((parsed_text, parsed_tool_intents)) = json_tool_call::extract_json_tool_call_turn(
        cleaned_text.as_str(),
        raw_meta,
        session_id,
        turn_id,
        bridge_context,
    ) {
        cleaned_text = parsed_text;
        tool_intents = parsed_tool_intents;
    }

    if tool_intents.is_empty() {
        match extract_invoke_block_turn(cleaned_text.as_str(), session_id, turn_id, bridge_context)
        {
            InvokeBlockParseResult::Parsed {
                cleaned_text: parsed_text,
                tool_intents: parsed_tool_intents,
                telemetry,
            } => {
                cleaned_text = parsed_text;
                tool_intents = parsed_tool_intents;
                attach_invoke_block_parse_telemetry(raw_meta, telemetry);
            }
            InvokeBlockParseResult::Malformed { telemetry } => {
                attach_invoke_block_parse_telemetry(raw_meta, telemetry);
            }
            InvokeBlockParseResult::Absent => {}
        }
    }

    if tool_intents.is_empty() {
        match extract_inline_function_call_turn(
            cleaned_text.as_str(),
            session_id,
            turn_id,
            bridge_context,
        ) {
            InlineFunctionParseResult::Parsed {
                cleaned_text: parsed_text,
                tool_intents: parsed_tool_intents,
                telemetry,
            } => {
                cleaned_text = parsed_text;
                tool_intents = parsed_tool_intents;
                attach_inline_function_parse_telemetry(raw_meta, telemetry);
            }
            InlineFunctionParseResult::Malformed { telemetry } => {
                attach_inline_function_parse_telemetry(raw_meta, telemetry);
            }
            InlineFunctionParseResult::Absent => {}
        }
    }

    OpenAiTextToolTurnExtraction {
        assistant_text: cleaned_text,
        tool_intents,
    }
}

pub(super) fn extract_message_content(body: &Value) -> Option<String> {
    if let Some(content) = extract_responses_message_content(body) {
        return Some(content);
    }

    if let Some(content) = extract_google_message_content(body) {
        return Some(content);
    }

    openai_message(body)
        .or_else(|| bedrock_message(body))
        .and_then(message_content_value)
        .or_else(|| body_content_value(body))
        .and_then(extract_content_text)
}

pub(super) fn extract_model_catalog_entries(body: &Value) -> Vec<super::ProviderModelCatalogEntry> {
    model_catalog::extract_model_catalog_entries(body)
}

fn message_content(message: &Value) -> Option<String> {
    message_content_value(message).and_then(extract_content_text)
}

fn message_content_value(message: &Value) -> Option<&Value> {
    message.get("content")
}

fn body_content_value(body: &Value) -> Option<&Value> {
    body.get("content")
}

fn openai_message(body: &Value) -> Option<&Value> {
    body.get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
}

fn bedrock_message(body: &Value) -> Option<&Value> {
    body.get("output").and_then(|output| output.get("message"))
}

fn google_message(body: &Value) -> Option<&Value> {
    body.get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
}

fn extract_google_message_content(body: &Value) -> Option<String> {
    google_message(body).and_then(google_message_content)
}

fn google_message_content(message: &Value) -> Option<String> {
    message.get("parts").and_then(extract_content_text)
}

fn extract_body_content_text(body: &Value) -> Option<String> {
    body_content_value(body).and_then(extract_content_text)
}

fn build_provider_tool_intent(
    raw_tool_name: &str,
    args_json: Value,
    source: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    tool_call_id: String,
    _bridge_context: &ProviderToolBridgeContext,
) -> Option<ToolIntent> {
    let canonical_tool_name = tools::canonical_tool_name(raw_tool_name).to_owned();
    let direct_tool_name =
        tools::direct_tool_name_for_hidden_tool(canonical_tool_name.as_str()).map(str::to_owned);
    let tool_name = direct_tool_name.unwrap_or(canonical_tool_name);
    let provider_visible = tools::is_provider_exposed_tool_name(tool_name.as_str());
    if !provider_visible {
        return None;
    }
    Some(ToolIntent {
        tool_name,
        args_json,
        source: source.to_owned(),
        session_id: session_id.unwrap_or_default().to_owned(),
        turn_id: turn_id.unwrap_or_default().to_owned(),
        tool_call_id,
    })
}

fn extract_openai_tool_intents(
    message: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Vec<ToolIntent> {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let raw_tool_name = function.get("name").and_then(Value::as_str)?;
                    let args_str = function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    let args_json = match serde_json::from_str::<Value>(args_str) {
                        Ok(value) => value,
                        Err(error) => json!({
                            "_parse_error": format!("{error}"),
                            "_raw_arguments": args_str
                        }),
                    };
                    let tool_call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    build_provider_tool_intent(
                        raw_tool_name,
                        args_json,
                        "provider_tool_call",
                        session_id,
                        turn_id,
                        tool_call_id,
                        bridge_context,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_anthropic_tool_intents(
    body: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Vec<ToolIntent> {
    body.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        return None;
                    }
                    let raw_tool_name = block.get("name").and_then(Value::as_str)?;
                    build_provider_tool_intent(
                        raw_tool_name,
                        block.get("input").cloned().unwrap_or_else(|| json!({})),
                        "provider_tool_call",
                        session_id,
                        turn_id,
                        block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        bridge_context,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_bedrock_tool_intents(
    message: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Vec<ToolIntent> {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    let tool_use = block.get("toolUse")?;
                    let raw_tool_name = tool_use.get("name").and_then(Value::as_str)?;
                    build_provider_tool_intent(
                        raw_tool_name,
                        tool_use.get("input").cloned().unwrap_or_else(|| json!({})),
                        "provider_tool_call",
                        session_id,
                        turn_id,
                        tool_use
                            .get("toolUseId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        bridge_context,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_google_tool_intents(
    message: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Vec<ToolIntent> {
    message
        .get("parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .enumerate()
                .filter_map(|(index, part)| {
                    let function_call = part.get("functionCall")?;
                    let raw_tool_name = function_call.get("name").and_then(Value::as_str)?;
                    let args_json = function_call
                        .get("args")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let tool_call_id = format!("google-call-{index}");
                    build_provider_tool_intent(
                        raw_tool_name,
                        args_json,
                        "provider_tool_call",
                        session_id,
                        turn_id,
                        tool_call_id,
                        bridge_context,
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_bedrock_message(message: &Value) -> Value {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("assistant");
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(normalize_bedrock_content_block)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "role": role,
        "content": content,
    })
}

fn normalize_bedrock_content_block(block: &Value) -> Option<Value> {
    if let Some(text) = block
        .get("text")
        .and_then(Value::as_str)
        .and_then(normalize_text)
    {
        return Some(json!({
            "type": "text",
            "text": text,
        }));
    }

    let tool_use = block.get("toolUse")?;
    let id = tool_use.get("toolUseId").and_then(Value::as_str)?;
    let name = tool_use.get("name").and_then(Value::as_str)?;
    Some(json!({
        "type": "tool_use",
        "id": id,
        "name": name,
        "input": tool_use.get("input").cloned().unwrap_or_else(|| json!({}))
    }))
}

fn extract_responses_provider_turn(
    body: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Option<ProviderTurn> {
    let output = response_output_items(body)?;
    let mut assistant_text = extract_responses_message_content(body).unwrap_or_default();
    let mut raw_meta = body.clone();
    let mut tool_intents = output
        .iter()
        .filter_map(|item| {
            response_tool_intent_from_item(item, session_id, turn_id, bridge_context)
        })
        .collect::<Vec<_>>();

    if tool_intents.is_empty() && !assistant_text.is_empty() {
        let extraction = extract_openai_text_tool_turn(
            assistant_text.as_str(),
            &mut raw_meta,
            session_id,
            turn_id,
            bridge_context,
        );
        assistant_text = extraction.assistant_text;
        tool_intents = extraction.tool_intents;
    }

    if assistant_text.is_empty() && tool_intents.is_empty() {
        return None;
    }

    Some(ProviderTurn {
        assistant_text,
        tool_intents,
        raw_meta,
    })
}

fn extract_responses_message_content(body: &Value) -> Option<String> {
    if let Some(text) = body.get("output_text").and_then(Value::as_str) {
        return normalize_text(text);
    }

    let output = response_output_items(body)?;
    let mut merged = Vec::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content") else {
            continue;
        };
        if let Some(text) = extract_content_text(content) {
            merged.push(text);
        }
    }

    if merged.is_empty() {
        return None;
    }
    normalize_text(&merged.join("\n"))
}

fn response_output_items(body: &Value) -> Option<&[Value]> {
    body.get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

fn response_tool_intent_from_item(
    item: &Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Option<ToolIntent> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if item_type != "function_call" && item_type != "tool_call" {
        return None;
    }

    let raw_tool_name = item.get("name").and_then(Value::as_str).or_else(|| {
        item.get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
    })?;
    let args_str = item
        .get("arguments")
        .and_then(Value::as_str)
        .or_else(|| {
            item.get("function")
                .and_then(|function| function.get("arguments"))
                .and_then(Value::as_str)
        })
        .unwrap_or("{}");
    let args_json = match serde_json::from_str::<Value>(args_str) {
        Ok(value) => value,
        Err(e) => json!({
            "_parse_error": format!("{e}"),
            "_raw_arguments": args_str
        }),
    };
    let tool_call_id = item
        .get("call_id")
        .and_then(Value::as_str)
        .or_else(|| item.get("id").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();

    build_provider_tool_intent(
        raw_tool_name,
        args_json,
        "provider_tool_call",
        session_id,
        turn_id,
        tool_call_id,
        bridge_context,
    )
}

fn extract_content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return normalize_text(text);
    }
    let parts = content.as_array()?;
    let mut merged = Vec::new();
    for part in parts {
        if let Some(text) = extract_content_part_text(part) {
            merged.push(text);
        }
    }
    if merged.is_empty() {
        return None;
    }
    normalize_text(&merged.join("\n"))
}

fn extract_content_part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return normalize_text(text);
    }
    if let Some(text) = part
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return normalize_text(text);
    }
    None
}

fn normalize_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InvokeBlockParseTelemetry {
    status: &'static str,
    tool_count: usize,
    error_code: Option<&'static str>,
}

impl InvokeBlockParseTelemetry {
    fn parsed(tool_count: usize) -> Self {
        Self {
            status: "parsed",
            tool_count,
            error_code: None,
        }
    }

    fn malformed(tool_count: usize, error_code: InvokeBlockParseError) -> Self {
        Self {
            status: "malformed",
            tool_count,
            error_code: Some(error_code.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
enum InvokeBlockParseResult {
    Parsed {
        cleaned_text: String,
        tool_intents: Vec<ToolIntent>,
        telemetry: InvokeBlockParseTelemetry,
    },
    Malformed {
        telemetry: InvokeBlockParseTelemetry,
    },
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvokeBlockParseError {
    MissingFunctionCallsClose,
    MissingInvokeOpen,
    MissingInvokeHeaderClose,
    MissingInvokeClose,
    MissingInvokeName,
    InvalidInvokeAttributes,
    InvalidArgumentsJson,
}

impl InvokeBlockParseError {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingFunctionCallsClose => "missing_function_calls_close",
            Self::MissingInvokeOpen => "missing_invoke_open",
            Self::MissingInvokeHeaderClose => "missing_invoke_header_close",
            Self::MissingInvokeClose => "missing_invoke_close",
            Self::MissingInvokeName => "missing_invoke_name",
            Self::InvalidInvokeAttributes => "invalid_invoke_attributes",
            Self::InvalidArgumentsJson => "invalid_arguments_json",
        }
    }
}

fn attach_invoke_block_parse_telemetry(raw_meta: &mut Value, telemetry: InvokeBlockParseTelemetry) {
    attach_provider_parse_telemetry(
        raw_meta,
        "invoke_block",
        telemetry.status,
        telemetry.tool_count,
        telemetry.error_code,
    );
}

fn attach_provider_parse_telemetry(
    raw_meta: &mut Value,
    key: &str,
    status: &str,
    tool_count: usize,
    error_code: Option<&str>,
) {
    const PROVIDER_PARSE_META_KEY: &str = "loong_provider_parse";
    const LEGACY_PROVIDER_PARSE_META_KEY: &str = "loong_provider_parse";

    let Some(message) = raw_meta.as_object_mut() else {
        return;
    };

    let mut entry = serde_json::Map::new();
    entry.insert("status".to_owned(), Value::String(status.to_owned()));
    entry.insert("tool_count".to_owned(), Value::from(tool_count as u64));
    if let Some(error_code) = error_code {
        entry.insert(
            "error_code".to_owned(),
            Value::String(error_code.to_owned()),
        );
    }

    for parse_key in [PROVIDER_PARSE_META_KEY, LEGACY_PROVIDER_PARSE_META_KEY] {
        let provider_parse = message
            .entry(parse_key.to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let Some(provider_parse) = provider_parse.as_object_mut() else {
            continue;
        };
        provider_parse.insert(key.to_owned(), Value::Object(entry.clone()));
    }
}

fn extract_invoke_block_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> InvokeBlockParseResult {
    const FUNCTION_CALLS_OPEN: &str = "<function_calls>";
    const FUNCTION_CALLS_CLOSE: &str = "</function_calls>";

    let mut cursor = 0usize;
    let mut cleaned = String::new();
    let mut tool_intents = Vec::new();
    let mut found_invoke_block = false;

    while let Some(relative_start) = text[cursor..].find(FUNCTION_CALLS_OPEN) {
        let start = cursor + relative_start;
        if !is_standalone_block_start(text, start)
            || is_inside_markdown_fence(text, start)
            || is_inside_markdown_indented_code_block(text, start)
        {
            let next_cursor = start + FUNCTION_CALLS_OPEN.len();
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
        }

        let body_start = start + FUNCTION_CALLS_OPEN.len();
        let body_remainder = &text[body_start..];
        let Some(body_end) = body_remainder.find(FUNCTION_CALLS_CLOSE) else {
            return InvokeBlockParseResult::Malformed {
                telemetry: InvokeBlockParseTelemetry::malformed(
                    tool_intents.len(),
                    InvokeBlockParseError::MissingFunctionCallsClose,
                ),
            };
        };
        let block_end = body_start + body_end + FUNCTION_CALLS_CLOSE.len();
        if !is_standalone_block_end(text, block_end) {
            cleaned.push_str(&text[cursor..block_end]);
            cursor = block_end;
            continue;
        }

        let block_body = &text[body_start..body_start + body_end];
        let parsed_tool_intents = match parse_invoke_block_sequence(
            block_body,
            session_id,
            turn_id,
            bridge_context,
            tool_intents.len(),
        ) {
            Ok(parsed_tool_intents) => parsed_tool_intents,
            Err(error_code) => {
                return InvokeBlockParseResult::Malformed {
                    telemetry: InvokeBlockParseTelemetry::malformed(tool_intents.len(), error_code),
                };
            }
        };

        if parsed_tool_intents.is_empty() {
            cleaned.push_str(&text[cursor..block_end]);
            cursor = block_end;
            continue;
        }

        found_invoke_block = true;
        cleaned.push_str(&text[cursor..start]);
        tool_intents.extend(parsed_tool_intents);
        cursor = block_end;
    }

    if !found_invoke_block {
        return InvokeBlockParseResult::Absent;
    }

    cleaned.push_str(&text[cursor..]);
    let tool_count = tool_intents.len();
    InvokeBlockParseResult::Parsed {
        cleaned_text: normalize_text(cleaned.as_str()).unwrap_or_default(),
        tool_intents,
        telemetry: InvokeBlockParseTelemetry::parsed(tool_count),
    }
}

fn parse_invoke_block_sequence(
    body: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_call_offset: usize,
) -> Result<Vec<ToolIntent>, InvokeBlockParseError> {
    const INVOKE_OPEN: &str = "<invoke";
    const INVOKE_CLOSE: &str = "</invoke>";

    let mut cursor = 0usize;
    let mut tool_intents = Vec::new();

    while cursor < body.len() {
        let remainder = &body[cursor..];
        let trimmed_len = remainder.len().saturating_sub(remainder.trim_start().len());
        cursor += trimmed_len;
        if cursor >= body.len() {
            break;
        }

        let remainder = &body[cursor..];
        if !remainder.starts_with(INVOKE_OPEN) {
            return Err(InvokeBlockParseError::MissingInvokeOpen);
        }

        let header_start = cursor + INVOKE_OPEN.len();
        let header_remainder = &body[header_start..];
        let Some(header_end) = find_unquoted_tag_close(header_remainder) else {
            return Err(InvokeBlockParseError::MissingInvokeHeaderClose);
        };
        let raw_header = &header_remainder[..header_end];
        let self_closing = raw_header.trim_end().ends_with('/');
        let normalized_header = raw_header.trim_end().trim_end_matches('/').trim();
        let attributes = parse_invoke_attributes(normalized_header)?;
        let raw_tool_name = attributes
            .get("name")
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or(InvokeBlockParseError::MissingInvokeName)?;

        let body_start = header_start + header_end + 1;
        let (invoke_body, invoke_end) = if self_closing {
            ("", body_start)
        } else {
            let invoke_remainder = &body[body_start..];
            let Some(invoke_end_relative) = invoke_remainder.find(INVOKE_CLOSE) else {
                return Err(InvokeBlockParseError::MissingInvokeClose);
            };
            let invoke_end = body_start + invoke_end_relative + INVOKE_CLOSE.len();
            (&invoke_remainder[..invoke_end_relative], invoke_end)
        };

        let canonical_tool_name = tools::canonical_tool_name(raw_tool_name).to_owned();
        let raw_arguments = attributes
            .get("arguments")
            .or_else(|| attributes.get("args"))
            .map(String::as_str)
            .unwrap_or(invoke_body);
        let args_json = parse_invoke_arguments(raw_arguments.trim())?;
        let tool_call_id = format!("invoke-call-{}", tool_call_offset + tool_intents.len());
        let tool_intent = build_provider_tool_intent(
            canonical_tool_name.as_str(),
            args_json,
            "provider_invoke_block_call",
            session_id,
            turn_id,
            tool_call_id,
            bridge_context,
        );
        if let Some(tool_intent) = tool_intent {
            tool_intents.push(tool_intent);
        }

        cursor = invoke_end;
    }

    Ok(tool_intents)
}

fn find_unquoted_tag_close(raw: &str) -> Option<usize> {
    let mut active_quote = None;
    let bytes = raw.as_bytes();

    for (index, ch) in raw.char_indices() {
        let is_escaped = quote_byte_is_escaped(bytes, index);

        if active_quote == Some(ch) && !is_escaped {
            active_quote = None;
            continue;
        }

        if active_quote.is_none() && !is_escaped && (ch == '"' || ch == '\'') {
            active_quote = Some(ch);
            continue;
        }

        if active_quote.is_none() && ch == '>' {
            return Some(index);
        }
    }

    None
}

fn quote_byte_is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut slash_count = 0usize;
    let mut cursor = index;

    while cursor > 0 {
        let previous_index = cursor - 1;
        let previous_byte = bytes.get(previous_index).copied();
        let Some(previous_byte) = previous_byte else {
            break;
        };
        if previous_byte != b'\\' {
            break;
        }

        slash_count += 1;
        cursor = previous_index;
    }

    slash_count % 2 == 1
}

fn parse_invoke_attributes(raw: &str) -> Result<BTreeMap<String, String>, InvokeBlockParseError> {
    let mut attributes = BTreeMap::new();
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;

    while cursor < raw.len() {
        while bytes
            .get(cursor)
            .copied()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if cursor >= raw.len() {
            break;
        }

        let name_start = cursor;
        while let Some(byte) = bytes.get(cursor).copied() {
            if byte.is_ascii_whitespace() || byte == b'=' {
                break;
            }
            cursor += 1;
        }
        if name_start == cursor {
            return Err(InvokeBlockParseError::InvalidInvokeAttributes);
        }
        let name = &raw[name_start..cursor];

        while bytes
            .get(cursor)
            .copied()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if bytes.get(cursor).copied() != Some(b'=') {
            return Err(InvokeBlockParseError::InvalidInvokeAttributes);
        }
        cursor += 1;
        while bytes
            .get(cursor)
            .copied()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        let Some(quote) = bytes.get(cursor).copied() else {
            return Err(InvokeBlockParseError::InvalidInvokeAttributes);
        };
        if !matches!(quote, b'"' | b'\'') {
            return Err(InvokeBlockParseError::InvalidInvokeAttributes);
        }
        cursor += 1;
        let value_start = cursor;
        while let Some(byte) = bytes.get(cursor).copied() {
            let is_closing_quote = byte == quote;
            let is_escaped = quote_byte_is_escaped(bytes, cursor);

            if is_closing_quote && !is_escaped {
                break;
            }

            cursor += 1;
        }
        if cursor >= raw.len() {
            return Err(InvokeBlockParseError::InvalidInvokeAttributes);
        }

        let value = decode_inline_xml_text(&raw[value_start..cursor]);
        attributes.insert(name.to_owned(), value);
        cursor += 1;
    }

    Ok(attributes)
}

fn parse_invoke_arguments(raw_arguments: &str) -> Result<Value, InvokeBlockParseError> {
    let decoded = decode_inline_xml_text(raw_arguments);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }

    let parsed = serde_json::from_str::<Value>(trimmed);
    if let Ok(value) = parsed {
        return Ok(value);
    }

    let backslash_unescaped = decode_backslash_escaped_quotes(trimmed);
    let reparsed = serde_json::from_str::<Value>(backslash_unescaped.as_str());
    if let Ok(value) = reparsed {
        return Ok(value);
    }

    Err(InvokeBlockParseError::InvalidArgumentsJson)
}

fn decode_backslash_escaped_quotes(raw: &str) -> String {
    let single_quotes_unescaped = raw.replace("\\'", "'");
    single_quotes_unescaped.replace("\\\"", "\"")
}

fn decode_inline_xml_text(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn is_standalone_block_start(text: &str, start: usize) -> bool {
    let line_start = text[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    text[line_start..start]
        .chars()
        .all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

fn is_standalone_block_end(text: &str, end: usize) -> bool {
    let line_end = text[end..]
        .find('\n')
        .map(|relative| end + relative)
        .unwrap_or(text.len());
    text[end..line_end]
        .chars()
        .all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

fn is_inside_markdown_fence(text: &str, index: usize) -> bool {
    let mut cursor = 0usize;
    let mut inside = false;
    let mut fence_marker = None;

    while cursor < index {
        let line_end = text[cursor..]
            .find('\n')
            .map(|relative| cursor + relative + 1)
            .unwrap_or(text.len());
        let line = &text[cursor..line_end];
        let trimmed = line.trim_start();

        if let Some(marker) = markdown_fence_marker(trimmed) {
            if inside {
                if fence_marker == Some(marker) {
                    inside = false;
                    fence_marker = None;
                }
            } else {
                inside = true;
                fence_marker = Some(marker);
            }
        }

        cursor = line_end;
    }

    inside
}

fn is_inside_markdown_indented_code_block(text: &str, index: usize) -> bool {
    let mut line_start = text[..index]
        .rfind('\n')
        .map(|offset| offset + 1)
        .unwrap_or(0);

    if !line_has_markdown_indented_code_prefix(&text[line_start..index]) {
        return false;
    }

    loop {
        if line_start == 0 {
            return true;
        }

        let previous_line_end = line_start.saturating_sub(1);
        let previous_line_start = text[..previous_line_end]
            .rfind('\n')
            .map(|offset| offset + 1)
            .unwrap_or(0);
        let previous_line = &text[previous_line_start..previous_line_end];

        if previous_line.trim().is_empty() {
            return true;
        }

        if !line_has_markdown_indented_code_prefix(previous_line) {
            return false;
        }

        line_start = previous_line_start;
    }
}

fn line_has_markdown_indented_code_prefix(line: &str) -> bool {
    let mut spaces = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => spaces += 1,
            '\t' => return true,
            '\r' => {}
            _ => return spaces >= 4,
        }
    }
    spaces >= 4
}

fn markdown_fence_marker(line: &str) -> Option<char> {
    if line.starts_with("```") {
        return Some('`');
    }
    if line.starts_with("~~~") {
        return Some('~');
    }
    None
}

#[cfg(test)]
mod tests;
