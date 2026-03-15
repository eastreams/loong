use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

use serde_json::{Value, json};

use crate::conversation::turn_engine::{ProviderTurn, ToolIntent};
use crate::tools;

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
        let mut tool_intents =
            extract_openai_tool_intents(message, session_id, turn_id, &bridge_context);

        if tool_intents.is_empty() {
            match extract_inline_function_call_turn(
                assistant_text.as_str(),
                session_id,
                turn_id,
                &bridge_context,
            ) {
                InlineFunctionParseResult::Parsed {
                    cleaned_text,
                    tool_intents: inline_tool_intents,
                    telemetry,
                } => {
                    assistant_text = cleaned_text;
                    tool_intents = inline_tool_intents;
                    attach_inline_function_parse_telemetry(&mut raw_meta, telemetry);
                }
                InlineFunctionParseResult::Malformed { telemetry } => {
                    attach_inline_function_parse_telemetry(&mut raw_meta, telemetry);
                }
                InlineFunctionParseResult::Absent => {}
            }
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
struct ProviderToolBridgeContext {
    discoverable_leases: BTreeMap<String, String>,
}

fn provider_tool_bridge_context_from_messages(messages: &[Value]) -> ProviderToolBridgeContext {
    messages
        .iter()
        .rev()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .and_then(parse_discovery_followup_leases_from_message_content)
        })
        .find(|context| !context.discoverable_leases.is_empty())
        .unwrap_or_default()
}

fn parse_discovery_followup_leases_from_message_content(
    content: &str,
) -> Option<ProviderToolBridgeContext> {
    let tool_result_text = content.trim().strip_prefix("[tool_result]\n")?;
    let mut discoverable_leases = BTreeMap::new();
    let catalog = tools::tool_catalog();

    for line in tool_result_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(payload) = trimmed.strip_prefix("[ok] ") else {
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if envelope.get("tool").and_then(Value::as_str) != Some("tool.search") {
            continue;
        }
        if envelope
            .get("payload_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(payload_summary) = envelope.get("payload_summary").and_then(Value::as_str) else {
            continue;
        };
        let Ok(payload_json) = serde_json::from_str::<Value>(payload_summary) else {
            continue;
        };
        let Some(results) = payload_json.get("results").and_then(Value::as_array) else {
            continue;
        };
        for result in results {
            let Some(tool_id) = result.get("tool_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(lease) = result.get("lease").and_then(Value::as_str) else {
                continue;
            };
            let canonical_tool_name = tools::canonical_tool_name(tool_id);
            let Some(descriptor) = catalog.descriptor(canonical_tool_name) else {
                continue;
            };
            if !descriptor.is_discoverable() {
                continue;
            }
            discoverable_leases
                .entry(descriptor.name.to_owned())
                .or_insert_with(|| lease.to_owned());
        }
    }

    (!discoverable_leases.is_empty()).then_some(ProviderToolBridgeContext {
        discoverable_leases,
    })
}

pub(super) fn extract_message_content(body: &Value) -> Option<String> {
    if let Some(content) = extract_responses_message_content(body) {
        return Some(content);
    }

    openai_message(body)
        .or_else(|| bedrock_message(body))
        .and_then(message_content_value)
        .or_else(|| body_content_value(body))
        .and_then(extract_content_text)
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
    bridge_context: &ProviderToolBridgeContext,
) -> ToolIntent {
    let canonical_tool_name = tools::canonical_tool_name(raw_tool_name).to_owned();
    let (tool_name, args_json) =
        match tools::tool_catalog().descriptor(canonical_tool_name.as_str()) {
            Some(descriptor) if descriptor.is_discoverable() => bridge_context
                .discoverable_leases
                .get(descriptor.name)
                .cloned()
                .map(|lease| {
                    (
                        "tool.invoke".to_owned(),
                        json!({
                            "tool_id": descriptor.name,
                            "lease": lease,
                            "arguments": args_json,
                        }),
                    )
                })
                .unwrap_or_else(|| (canonical_tool_name, args_json)),
            _ => (canonical_tool_name, args_json),
        };
    ToolIntent {
        tool_name,
        args_json,
        source: source.to_owned(),
        session_id: session_id.unwrap_or_default().to_owned(),
        turn_id: turn_id.unwrap_or_default().to_owned(),
        tool_call_id,
    }
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
                    Some(build_provider_tool_intent(
                        raw_tool_name,
                        args_json,
                        "provider_tool_call",
                        session_id,
                        turn_id,
                        tool_call_id,
                        bridge_context,
                    ))
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
                    Some(build_provider_tool_intent(
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
                    ))
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
                    Some(build_provider_tool_intent(
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
                    ))
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
    let assistant_text = extract_responses_message_content(body).unwrap_or_default();
    let tool_intents = output
        .iter()
        .filter_map(|item| {
            response_tool_intent_from_item(item, session_id, turn_id, bridge_context)
        })
        .collect::<Vec<_>>();

    if assistant_text.is_empty() && tool_intents.is_empty() {
        return None;
    }

    Some(ProviderTurn {
        assistant_text,
        tool_intents,
        raw_meta: body.clone(),
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

    Some(build_provider_tool_intent(
        raw_tool_name,
        args_json,
        "provider_tool_call",
        session_id,
        turn_id,
        tool_call_id,
        bridge_context,
    ))
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
struct InlineFunctionParseTelemetry {
    status: &'static str,
    tool_count: usize,
    error_code: Option<&'static str>,
}

impl InlineFunctionParseTelemetry {
    fn parsed(tool_count: usize) -> Self {
        Self {
            status: "parsed",
            tool_count,
            error_code: None,
        }
    }

    fn malformed(tool_count: usize, error_code: InlineFunctionParseError) -> Self {
        Self {
            status: "malformed",
            tool_count,
            error_code: Some(error_code.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
enum InlineFunctionParseResult {
    Parsed {
        cleaned_text: String,
        tool_intents: Vec<ToolIntent>,
        telemetry: InlineFunctionParseTelemetry,
    },
    Malformed {
        telemetry: InlineFunctionParseTelemetry,
    },
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineFunctionParseError {
    MissingFunctionHeaderClose,
    EmptyFunctionName,
    MissingFunctionClose,
    MissingParameterOpen,
    MissingParameterHeaderClose,
    EmptyParameterName,
    MissingParameterClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineParameterSchemaType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
}

impl InlineParameterSchemaType {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "string" => Some(Self::String),
            "integer" => Some(Self::Integer),
            "number" => Some(Self::Number),
            "boolean" => Some(Self::Boolean),
            "array" => Some(Self::Array),
            "object" => Some(Self::Object),
            _ => None,
        }
    }
}

impl InlineFunctionParseError {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingFunctionHeaderClose => "missing_function_header_close",
            Self::EmptyFunctionName => "empty_function_name",
            Self::MissingFunctionClose => "missing_function_close",
            Self::MissingParameterOpen => "missing_parameter_open",
            Self::MissingParameterHeaderClose => "missing_parameter_header_close",
            Self::EmptyParameterName => "empty_parameter_name",
            Self::MissingParameterClose => "missing_parameter_close",
        }
    }
}

fn attach_inline_function_parse_telemetry(
    raw_meta: &mut Value,
    telemetry: InlineFunctionParseTelemetry,
) {
    let Some(message) = raw_meta.as_object_mut() else {
        return;
    };

    let mut inline_function = serde_json::Map::new();
    inline_function.insert(
        "status".to_owned(),
        Value::String(telemetry.status.to_owned()),
    );
    inline_function.insert(
        "tool_count".to_owned(),
        Value::from(telemetry.tool_count as u64),
    );
    if let Some(error_code) = telemetry.error_code {
        inline_function.insert(
            "error_code".to_owned(),
            Value::String(error_code.to_owned()),
        );
    }

    let mut provider_parse = serde_json::Map::new();
    provider_parse.insert("inline_function".to_owned(), Value::Object(inline_function));
    message.insert(
        "loongclaw_provider_parse".to_owned(),
        Value::Object(provider_parse),
    );
}

fn extract_inline_function_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> InlineFunctionParseResult {
    const FUNCTION_OPEN: &str = "<function=";
    const FUNCTION_CLOSE: &str = "</function>";

    let mut cursor = 0usize;
    let mut cleaned = String::new();
    let mut tool_intents = Vec::new();
    let mut found_inline_function = false;

    while let Some(relative_start) = text[cursor..].find(FUNCTION_OPEN) {
        let start = cursor + relative_start;
        if !is_standalone_inline_function_start(text, start)
            || is_inside_markdown_fence(text, start)
            || is_inside_markdown_indented_code_block(text, start)
        {
            let next_cursor = start + FUNCTION_OPEN.len();
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
        }

        let name_start = start + FUNCTION_OPEN.len();
        let header_remainder = &text[name_start..];
        let Some(header_end) = header_remainder.find('>') else {
            return InlineFunctionParseResult::Malformed {
                telemetry: InlineFunctionParseTelemetry::malformed(
                    tool_intents.len(),
                    InlineFunctionParseError::MissingFunctionHeaderClose,
                ),
            };
        };
        let raw_tool_name = header_remainder[..header_end].trim();
        if raw_tool_name.is_empty() {
            return InlineFunctionParseResult::Malformed {
                telemetry: InlineFunctionParseTelemetry::malformed(
                    tool_intents.len(),
                    InlineFunctionParseError::EmptyFunctionName,
                ),
            };
        }

        let body_start = name_start + header_end + 1;
        let body_remainder = &text[body_start..];
        let Some(body_end) = body_remainder.find(FUNCTION_CLOSE) else {
            return InlineFunctionParseResult::Malformed {
                telemetry: InlineFunctionParseTelemetry::malformed(
                    tool_intents.len(),
                    InlineFunctionParseError::MissingFunctionClose,
                ),
            };
        };
        let function_body = &body_remainder[..body_end];
        let function_end = body_start + body_end + FUNCTION_CLOSE.len();
        if !is_standalone_inline_function_end(text, function_end) {
            cleaned.push_str(&text[cursor..function_end]);
            cursor = function_end;
            continue;
        }

        let canonical_tool_name = tools::canonical_tool_name(raw_tool_name).to_owned();
        let args_json =
            match parse_inline_function_parameters(canonical_tool_name.as_str(), function_body) {
                Ok(args_json) => args_json,
                Err(error_code) => {
                    return InlineFunctionParseResult::Malformed {
                        telemetry: InlineFunctionParseTelemetry::malformed(
                            tool_intents.len(),
                            error_code,
                        ),
                    };
                }
            };

        found_inline_function = true;
        cleaned.push_str(&text[cursor..start]);
        let tool_call_id = format!("inline-call-{}", tool_intents.len());
        tool_intents.push(build_provider_tool_intent(
            canonical_tool_name.as_str(),
            args_json,
            "provider_inline_function_call",
            session_id,
            turn_id,
            tool_call_id,
            bridge_context,
        ));

        cursor = function_end;
    }

    if !found_inline_function {
        return InlineFunctionParseResult::Absent;
    }

    cleaned.push_str(&text[cursor..]);
    let telemetry = InlineFunctionParseTelemetry::parsed(tool_intents.len());
    InlineFunctionParseResult::Parsed {
        cleaned_text: normalize_text(cleaned.as_str()).unwrap_or_default(),
        tool_intents,
        telemetry,
    }
}

fn parse_inline_function_parameters(
    tool_name: &str,
    body: &str,
) -> Result<Value, InlineFunctionParseError> {
    const PARAMETER_OPEN: &str = "<parameter=";
    const PARAMETER_CLOSE: &str = "</parameter>";

    let mut cursor = 0usize;
    let mut payload = serde_json::Map::new();

    while cursor < body.len() {
        let remainder = &body[cursor..];
        let trimmed_len = remainder.len().saturating_sub(remainder.trim_start().len());
        cursor += trimmed_len;
        if cursor >= body.len() {
            break;
        }

        let remainder = &body[cursor..];
        if !remainder.starts_with(PARAMETER_OPEN) {
            return Err(InlineFunctionParseError::MissingParameterOpen);
        }

        let name_start = cursor + PARAMETER_OPEN.len();
        let name_remainder = &body[name_start..];
        let Some(name_end) = name_remainder.find('>') else {
            return Err(InlineFunctionParseError::MissingParameterHeaderClose);
        };
        let parameter_name = name_remainder[..name_end].trim();
        if parameter_name.is_empty() {
            return Err(InlineFunctionParseError::EmptyParameterName);
        }

        let value_start = name_start + name_end + 1;
        let value_remainder = &body[value_start..];
        let Some(value_end) = value_remainder.find(PARAMETER_CLOSE) else {
            return Err(InlineFunctionParseError::MissingParameterClose);
        };
        let raw_value = &value_remainder[..value_end];
        payload.insert(
            parameter_name.to_owned(),
            parse_inline_parameter_value(tool_name, parameter_name, raw_value),
        );

        cursor = value_start + value_end + PARAMETER_CLOSE.len();
    }

    Ok(Value::Object(payload))
}

fn parse_inline_parameter_value(tool_name: &str, parameter_name: &str, raw_value: &str) -> Value {
    let decoded = decode_inline_xml_text(raw_value);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    match inline_parameter_schema_type(tool_name, parameter_name) {
        Some(InlineParameterSchemaType::String) => parse_inline_string_value(trimmed),
        Some(
            InlineParameterSchemaType::Integer
            | InlineParameterSchemaType::Number
            | InlineParameterSchemaType::Boolean
            | InlineParameterSchemaType::Array
            | InlineParameterSchemaType::Object,
        )
        | None => serde_json::from_str::<Value>(trimmed)
            .unwrap_or_else(|_| Value::String(trimmed.to_owned())),
    }
}

fn decode_inline_xml_text(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_inline_string_value(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::String(value)) => Value::String(value),
        _ => Value::String(raw.to_owned()),
    }
}

fn inline_parameter_schema_type(
    tool_name: &str,
    parameter_name: &str,
) -> Option<InlineParameterSchemaType> {
    inline_parameter_schema_types()
        .get(tool_name)
        .and_then(|parameters| parameters.get(parameter_name))
        .copied()
}

fn inline_parameter_schema_types()
-> &'static BTreeMap<String, BTreeMap<String, InlineParameterSchemaType>> {
    static SCHEMA_TYPES: OnceLock<BTreeMap<String, BTreeMap<String, InlineParameterSchemaType>>> =
        OnceLock::new();

    SCHEMA_TYPES.get_or_init(|| {
        let mut tools_by_name =
            BTreeMap::<String, BTreeMap<String, InlineParameterSchemaType>>::new();
        for (tool_name, properties) in tools::tool_parameter_schema_types() {
            let entry = tools_by_name.entry(tool_name).or_default();
            for (parameter_name, schema_type) in properties {
                let Some(parameter_type) = InlineParameterSchemaType::parse(schema_type) else {
                    continue;
                };
                entry.insert(parameter_name, parameter_type);
            }
        }
        tools_by_name
    })
}

fn is_standalone_inline_function_start(text: &str, start: usize) -> bool {
    let line_start = text[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    text[line_start..start]
        .chars()
        .all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

fn is_standalone_inline_function_end(text: &str, end: usize) -> bool {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelCandidate {
    id: String,
    created: Option<i64>,
    created_text: Option<String>,
    deprecated: bool,
}

pub(super) fn extract_model_ids(body: &Value) -> Vec<String> {
    let mut candidates = collect_model_candidates(body);
    if candidates.is_empty() {
        return Vec::new();
    }

    candidates.sort_by(|left, right| {
        left.deprecated
            .cmp(&right.deprecated)
            .then_with(|| {
                right
                    .created
                    .cmp(&left.created)
                    .then_with(|| right.created_text.cmp(&left.created_text))
            })
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for candidate in candidates {
        if seen.insert(candidate.id.clone()) {
            ids.push(candidate.id);
        }
    }
    ids
}

fn collect_model_candidates(body: &Value) -> Vec<ModelCandidate> {
    let mut out = Vec::new();
    let Some(items) = model_items(body) else {
        return out;
    };

    for item in items {
        if model_is_known_non_chat_candidate(item) {
            continue;
        }
        if let Some(id) = model_id_from_value(item) {
            out.push(ModelCandidate {
                id,
                created: model_created_from_value(item),
                created_text: model_created_text_from_value(item),
                deprecated: model_is_deprecated(item),
            });
        }
    }
    out
}

fn model_items(body: &Value) -> Option<&[Value]> {
    if let Some(data) = body.get("data").and_then(Value::as_array) {
        return Some(data);
    }
    if let Some(models) = body.get("modelSummaries").and_then(Value::as_array) {
        return Some(models);
    }
    if let Some(models) = body.get("models").and_then(Value::as_array) {
        return Some(models);
    }
    if let Some(models) = body
        .get("Result")
        .and_then(|value| value.get("Items"))
        .and_then(Value::as_array)
    {
        return Some(models);
    }
    if let Some(models) = body
        .get("result")
        .and_then(|value| value.get("models"))
        .and_then(Value::as_array)
    {
        return Some(models);
    }
    body.as_array().map(Vec::as_slice)
}

fn model_id_from_value(value: &Value) -> Option<String> {
    if let Some(id) = value.as_str() {
        return normalize_text(id);
    }
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("modelId").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("model").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("name").and_then(Value::as_str) {
        return normalize_text(id);
    }
    None
}

fn model_is_known_non_chat_candidate(value: &Value) -> bool {
    if model_has_explicit_non_chat_endpoint_compatibility(value) {
        return true;
    }

    if model_has_explicit_non_chat_completion_capability(value) {
        return true;
    }

    if model_is_archived(value) {
        return true;
    }

    if model_has_explicit_non_text_output_capability(value) {
        return true;
    }

    false
}

fn model_has_explicit_non_chat_endpoint_compatibility(value: &Value) -> bool {
    let Some(array) = value
        .get("supportedEndpointTypes")
        .or_else(|| value.get("supported_endpoint_types"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let endpoints = array
        .iter()
        .filter_map(Value::as_str)
        .map(|entry| entry.to_ascii_lowercase())
        .collect::<Vec<_>>();
    !endpoints.is_empty()
        && !endpoints.iter().any(|entry| {
            matches!(
                entry.as_str(),
                "chat" | "chat_completion" | "chat-completion"
            )
        })
}

fn model_has_explicit_non_chat_completion_capability(value: &Value) -> bool {
    if value
        .get("supports_chat")
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled)
    {
        return true;
    }
    if value
        .get("chat_completion")
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled)
    {
        return true;
    }
    false
}

fn model_is_archived(value: &Value) -> bool {
    value
        .get("archived")
        .and_then(Value::as_bool)
        .or_else(|| value.get("is_archived").and_then(Value::as_bool))
        == Some(true)
}

fn model_has_explicit_non_text_output_capability(value: &Value) -> bool {
    let Some(output_modalities) = value
        .get("output_modalities")
        .or_else(|| value.get("outputModalities"))
        .and_then(Value::as_array)
    else {
        return false;
    };

    let modalities = output_modalities
        .iter()
        .filter_map(Value::as_str)
        .map(|entry| entry.to_ascii_lowercase())
        .collect::<Vec<_>>();
    !modalities.is_empty() && !modalities.iter().any(|entry| entry == "text")
}

fn model_created_from_value(value: &Value) -> Option<i64> {
    if let Some(created) = value.get("created").and_then(Value::as_i64) {
        return Some(created);
    }
    if let Some(created) = value.get("created").and_then(Value::as_u64) {
        return i64::try_from(created).ok();
    }
    if let Some(created) = value.get("created_at").and_then(Value::as_i64) {
        return Some(created);
    }
    if let Some(created) = value.get("created_at").and_then(Value::as_u64) {
        return i64::try_from(created).ok();
    }
    None
}

fn model_created_text_from_value(value: &Value) -> Option<String> {
    for key in ["created_at", "createdAt", "release_date", "releaseDate"] {
        if let Some(text) = value.get(key).and_then(Value::as_str)
            && let Some(normalized) = normalize_text(text)
        {
            return Some(normalized);
        }
    }
    None
}

fn model_is_deprecated(value: &Value) -> bool {
    if value
        .get("deprecated")
        .and_then(Value::as_bool)
        .is_some_and(|deprecated| deprecated)
    {
        return true;
    }
    if value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "deprecated" | "deprecation" | "retired" | "sunset"
            )
        })
    {
        return true;
    }
    if let Some(tags) = value.get("tags").and_then(Value::as_array) {
        let normalized = tags
            .iter()
            .filter_map(Value::as_str)
            .map(|entry| entry.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if normalized.contains("deprecated") || normalized.contains("retired") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn discovery_followup_messages(tool_id: &str, lease: &str) -> Vec<Value> {
        let payload_summary = serde_json::to_string(&json!({
            "results": [
                {
                    "tool_id": tool_id,
                    "lease": lease,
                }
            ]
        }))
        .expect("encode search payload summary");
        let envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": payload_summary.chars().count(),
            "payload_truncated": false,
        }))
        .expect("encode search envelope");
        vec![json!({
            "role": "assistant",
            "content": format!("[tool_result]\n[ok] {envelope}"),
        })]
    }

    #[test]
    fn extract_provider_turn_parses_tool_calls() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "checking",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "file.read",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "file.read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
        assert_eq!(turn.tool_intents[0].tool_call_id, "call_1");
    }

    #[test]
    fn extract_provider_turn_surfaces_malformed_json_args() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "calling",
                    "tool_calls": [{
                        "id": "call_bad",
                        "type": "function",
                        "function": {
                            "name": "file.read",
                            "arguments": "{{not valid json"
                        }
                    }]
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.tool_intents.len(), 1);
        let args = &turn.tool_intents[0].args_json;
        assert!(
            args.get("_parse_error").is_some(),
            "malformed args should surface parse error, got: {args}"
        );
        assert_eq!(
            args.get("_raw_arguments").and_then(|v| v.as_str()),
            Some("{{not valid json")
        );
    }

    #[test]
    fn extract_provider_turn_normalizes_underscore_tool_aliases() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "calling",
                    "tool_calls": [{
                        "id": "call_underscore",
                        "type": "function",
                        "function": {
                            "name": "file_read",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "file.read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
    }

    #[test]
    fn extract_provider_turn_with_scope_rewrites_discoverable_tools_to_tool_invoke_after_search() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "checking",
                    "tool_calls": [{
                        "id": "call_compat",
                        "type": "function",
                        "function": {
                            "name": "file.read",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        });
        let messages = discovery_followup_messages("file.read", "lease-openai");

        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-shape"),
            Some("turn-shape"),
            &messages,
        )
        .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].session_id, "session-shape");
        assert_eq!(turn.tool_intents[0].turn_id, "turn-shape");
        assert_eq!(turn.tool_intents[0].tool_call_id, "call_compat");
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "file.read");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"path":"README.md"})
        );
        assert_eq!(turn.tool_intents[0].args_json["lease"], "lease-openai");
    }

    #[test]
    fn extract_provider_turn_handles_text_only() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "hello world"
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "hello world");
        assert!(turn.tool_intents.is_empty());
    }

    #[test]
    fn extract_provider_turn_supports_responses_function_calls() {
        let body = serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Reading the file."}
                    ]
                },
                {
                    "type": "function_call",
                    "name": "file_read",
                    "arguments": "{\"path\":\"README.md\"}",
                    "call_id": "call_resp_1"
                }
            ]
        });
        let messages = discovery_followup_messages("file.read", "lease-responses");
        let turn = extract_provider_turn_with_scope(
            &body,
            Some("session-responses"),
            Some("turn-responses"),
        )
        .expect("responses turn without search context should stay direct");
        assert_eq!(turn.assistant_text, "Reading the file.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "file.read");
        assert_eq!(turn.tool_intents[0].session_id, "session-responses");
        assert_eq!(turn.tool_intents[0].turn_id, "turn-responses");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
        assert_eq!(turn.tool_intents[0].tool_call_id, "call_resp_1");

        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-responses"),
            Some("turn-responses"),
            &messages,
        )
        .expect("responses turn with search context");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "file.read");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"path": "README.md"})
        );
        assert_eq!(turn.tool_intents[0].args_json["lease"], "lease-responses");
    }

    #[test]
    fn extract_provider_turn_parses_inline_shell_function_block() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "sorry, that command failed. let me retry with a simpler approach:\n<function=shell.exec><parameter=command>ls /root</parameter></function>"
                }
            }]
        });
        let messages = discovery_followup_messages("shell.exec", "lease-shell-inline");

        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(
            turn.assistant_text,
            "sorry, that command failed. let me retry with a simpler approach:"
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "shell.exec");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"command":"ls /root"})
        );
        assert_eq!(
            turn.tool_intents[0].args_json["lease"],
            "lease-shell-inline"
        );
        assert_eq!(
            turn.raw_meta["loongclaw_provider_parse"]["inline_function"]["status"],
            "parsed"
        );
        assert_eq!(
            turn.raw_meta["loongclaw_provider_parse"]["inline_function"]["tool_count"],
            1
        );
    }

    #[test]
    fn extract_provider_turn_parses_inline_external_skill_function_block() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "i can see the Home Assistant skill is installed. let me call it to fetch all entity states.\n<function=external_skills.invoke><parameter=skill_id>home-assistant-1-0-0</parameter><parameter=action>get_states</parameter></function>"
                }
            }]
        });
        let messages =
            discovery_followup_messages("external_skills.invoke", "lease-external-skill-inline");

        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(
            turn.assistant_text,
            "i can see the Home Assistant skill is installed. let me call it to fetch all entity states."
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(
            turn.tool_intents[0].args_json["tool_id"],
            "external_skills.invoke"
        );
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"skill_id":"home-assistant-1-0-0","action":"get_states"})
        );
        assert_eq!(
            turn.tool_intents[0].args_json["lease"],
            "lease-external-skill-inline"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_literal_inline_function_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "if you want to invoke it manually, you can write it like ` <function=shell.exec><parameter=command>ls</parameter></function> `."
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "if you want to invoke it manually, you can write it like ` <function=shell.exec><parameter=command>ls</parameter></function> `."
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_fenced_inline_function_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n```xml\n<function=shell.exec><parameter=command>ls</parameter></function>\n```"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n```xml\n<function=shell.exec><parameter=command>ls</parameter></function>\n```"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_indented_code_block_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n\n    <function=shell.exec><parameter=command>ls</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n\n    <function=shell.exec><parameter=command>ls</parameter></function>"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_multiline_indented_code_block_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n\n    step one\n    <function=shell.exec><parameter=command>ls</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n\n    step one\n    <function=shell.exec><parameter=command>ls</parameter></function>"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_tab_indented_code_block_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
        );
    }

    #[test]
    fn extract_provider_turn_parses_indented_inline_function_when_not_code_block() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me retry:\n    <function=shell.exec><parameter=command>ls</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "let me retry:");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "shell.exec");
        assert_eq!(turn.tool_intents[0].args_json, json!({"command": "ls"}));
    }

    #[test]
    fn extract_provider_turn_parses_tab_indented_inline_function_when_not_code_block() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me retry:\n\t<function=shell.exec><parameter=command>ls</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "let me retry:");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "shell.exec");
        assert_eq!(turn.tool_intents[0].args_json, json!({"command": "ls"}));
    }

    #[test]
    fn extract_provider_turn_recovers_inline_parameter_json_types() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me retry with structured parameters.\n<function=shell.exec><parameter=command>\"echo\"</parameter><parameter=args>[\"hello\",\"world\"]</parameter><parameter=timeout_ms>3000</parameter><parameter=login>false</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "echo",
                "args": ["hello", "world"],
                "timeout_ms": 3000,
                "login": false
            })
        );
    }

    #[test]
    fn extract_provider_turn_preserves_string_typed_inline_parameters() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me retry.\n<function=shell.exec><parameter=command>true</parameter><parameter=args>[\"hello\"]</parameter></function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "true",
                "args": ["hello"]
            })
        );
    }

    #[test]
    fn extract_provider_turn_records_malformed_inline_function_telemetry() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me retry.\n<function=shell.exec><parameter=command>ls /root</parameter>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(
            turn.assistant_text,
            "let me retry.\n<function=shell.exec><parameter=command>ls /root</parameter>"
        );
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.raw_meta["loongclaw_provider_parse"]["inline_function"]["status"],
            "malformed"
        );
        assert_eq!(
            turn.raw_meta["loongclaw_provider_parse"]["inline_function"]["error_code"],
            "missing_function_close"
        );
    }

    #[test]
    fn extract_provider_turn_supports_array_content_shape() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "line1"},
                        {"type": "text", "text": {"value": "line2"}}
                    ]
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "line1\nline2");
        assert!(turn.tool_intents.is_empty());
    }

    #[test]
    fn extract_provider_turn_preserves_reasoning_content_in_raw_meta() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "done",
                    "reasoning_content": "thinking"
                }
            }]
        });
        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "done");
        assert_eq!(turn.raw_meta["reasoning_content"], "thinking");
    }

    #[test]
    fn extract_provider_turn_supports_anthropic_native_content_blocks() {
        let body = json!({
            "content": [
                {
                    "type": "text",
                    "text": "checking"
                },
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "file_read",
                    "input": {
                        "path": "README.md"
                    }
                }
            ]
        });
        let messages = discovery_followup_messages("file.read", "lease-anthropic");
        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "file.read");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"]["path"],
            "README.md"
        );
        assert_eq!(turn.tool_intents[0].args_json["lease"], "lease-anthropic");
    }

    #[test]
    fn extract_provider_turn_supports_bedrock_converse_content_blocks() {
        let body = json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "text": "checking"
                        },
                        {
                            "toolUse": {
                                "toolUseId": "toolu_1",
                                "name": "file_read",
                                "input": {
                                    "path": "README.md"
                                }
                            }
                        }
                    ]
                }
            },
            "stopReason": "tool_use"
        });
        let messages = discovery_followup_messages("file.read", "lease-bedrock");
        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "file.read");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"]["path"],
            "README.md"
        );
        assert_eq!(turn.tool_intents[0].args_json["lease"], "lease-bedrock");
        assert_eq!(turn.raw_meta["content"][1]["type"], "tool_use");
        assert_eq!(turn.raw_meta["content"][1]["id"], "toolu_1");
    }

    #[test]
    fn extract_message_content_supports_part_array_shape() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "line1"},
                        {"type": "text", "text": {"value": "line2"}}
                    ]
                }
            }]
        });
        let content = extract_message_content(&body).expect("content");
        assert_eq!(content, "line1\nline2");
    }

    #[test]
    fn extract_message_content_keeps_plain_string_shape() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "  hello world  "
                }
            }]
        });
        let content = extract_message_content(&body).expect("content");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn extract_message_content_supports_responses_output_shape() {
        let body = json!({
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [
                    {"type": "output_text", "text": "line1"},
                    {"type": "output_text", "text": {"value": "line2"}}
                ]
            }]
        });
        let content = extract_message_content(&body).expect("responses content");
        assert_eq!(content, "line1\nline2");
    }

    #[test]
    fn extract_message_content_ignores_empty_parts() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "   "},
                        {"type": "text", "text": {"value": ""}}
                    ]
                }
            }]
        });
        assert!(extract_message_content(&body).is_none());
    }

    #[test]
    fn extract_model_ids_prefers_newer_timestamp_when_available() {
        let body = json!({
            "data": [
                {"id": "model-v1", "created": 100},
                {"id": "model-v2", "created": 200}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-v2", "model-v1"]);
    }

    #[test]
    fn extract_model_ids_supports_models_array_and_strings() {
        let body = json!({
            "models": [
                "model-c",
                {"name": "model-b"},
                {"model": "model-a"}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-a", "model-b", "model-c"]);
    }

    #[test]
    fn extract_model_ids_supports_bedrock_model_summaries() {
        let body = json!({
            "modelSummaries": [
                {
                    "modelId": "amazon.nova-lite-v1:0",
                    "modelName": "Nova Lite",
                    "providerName": "Amazon"
                },
                {
                    "modelId": "anthropic.claude-3-7-sonnet-20250219-v1:0",
                    "modelName": "Claude 3.7 Sonnet",
                    "providerName": "Anthropic"
                }
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(
            ids,
            vec![
                "amazon.nova-lite-v1:0",
                "anthropic.claude-3-7-sonnet-20250219-v1:0"
            ]
        );
    }

    #[test]
    fn extract_model_ids_deduplicates_results() {
        let body = json!({
            "data": [
                {"id": "model-a", "created": 200},
                {"id": "model-a", "created": 100},
                {"id": "model-b", "created": 150}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-a", "model-b"]);
    }
}
