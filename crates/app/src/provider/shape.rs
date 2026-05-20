use serde_json::{Value, json};

use crate::conversation::turn_engine::{ProviderTurn, ToolIntent};
use crate::tools;

mod inline_function;
mod invoke_block;
mod json_tool_call;
mod model_catalog;

use inline_function::{
    InlineFunctionParseResult, attach_inline_function_parse_telemetry,
    extract_inline_function_call_turn,
};
use invoke_block::{
    InvokeBlockParseResult, attach_invoke_block_parse_telemetry, extract_invoke_block_turn,
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

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn extract_model_ids(body: &Value) -> Vec<String> {
    model_catalog::extract_model_ids(body)
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

pub(super) fn decode_inline_xml_text(raw: &str) -> String {
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
