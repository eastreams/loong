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
struct ProviderToolBridgeContext {
    discoverable_leases: BTreeMap<String, String>,
}

struct OpenAiTextToolTurnExtraction {
    assistant_text: String,
    tool_intents: Vec<ToolIntent>,
}

fn provider_tool_bridge_context_from_messages(messages: &[Value]) -> ProviderToolBridgeContext {
    messages
        .iter()
        .rev()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|message| {
            let content = message.get("content")?;
            let content_text = extract_content_text(content)?;
            parse_discovery_followup_leases_from_message_content(content_text.as_str())
        })
        .find(|context| !context.discoverable_leases.is_empty())
        .unwrap_or_default()
}

fn parse_discovery_followup_leases_from_message_content(
    content: &str,
) -> Option<ProviderToolBridgeContext> {
    let tool_result_text = content.trim().strip_prefix("[tool_result]\n")?;
    let mut discoverable_leases = BTreeMap::new();

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
            let Some(discoverable_tool_name) = discoverable_tool_name(tool_id) else {
                continue;
            };
            discoverable_leases.insert(discoverable_tool_name.to_owned(), lease.to_owned());
        }
    }

    (!discoverable_leases.is_empty()).then_some(ProviderToolBridgeContext {
        discoverable_leases,
    })
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

    match extract_json_tool_call_turn(cleaned_text.as_str(), session_id, turn_id, bridge_context) {
        JsonToolBlockParseResult::Parsed {
            cleaned_text: parsed_text,
            tool_intents: parsed_tool_intents,
            telemetry,
        } => {
            cleaned_text = parsed_text;
            tool_intents = parsed_tool_intents;
            attach_json_tool_block_parse_telemetry(raw_meta, telemetry);
        }
        JsonToolBlockParseResult::Malformed { telemetry } => {
            attach_json_tool_block_parse_telemetry(raw_meta, telemetry);
        }
        JsonToolBlockParseResult::Absent => {}
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
    bridge_context: &ProviderToolBridgeContext,
) -> ToolIntent {
    let canonical_tool_name = tools::canonical_tool_name(raw_tool_name).to_owned();

    let normalized_direct_tool_intent = (canonical_tool_name == "tool.invoke")
        .then(|| direct_tool_intent_from_invoke_payload(&args_json))
        .flatten();
    let (tool_name, args_json) = if let Some((tool_name, arguments)) = normalized_direct_tool_intent
    {
        (tool_name, arguments)
    } else {
        let direct_tool_name =
            tools::direct_tool_name_for_hidden_tool(canonical_tool_name.as_str())
                .map(str::to_owned);
        let hidden_tool_name = direct_tool_name
            .is_none()
            .then(|| discoverable_tool_name(canonical_tool_name.as_str()))
            .flatten();
        let hidden_operation = tools::hidden_operation_for_tool_name(canonical_tool_name.as_str());
        let leased_hidden_tool_call = hidden_tool_name.and_then(|discoverable_tool_name| {
            bridge_context
                .discoverable_leases
                .get(discoverable_tool_name)
                .cloned()
                .map(|lease| {
                    let mut arguments = args_json.clone();
                    if let Some(operation) = hidden_operation.as_deref()
                        && let Some(arguments_object) = arguments.as_object_mut()
                    {
                        arguments_object
                            .entry("operation".to_owned())
                            .or_insert_with(|| json!(operation));
                    }
                    (
                        "tool.invoke".to_owned(),
                        json!({
                            "tool_id": discoverable_tool_name,
                            "lease": lease,
                            "arguments": arguments,
                        }),
                    )
                })
        });
        match direct_tool_name {
            Some(direct_tool_name) => (direct_tool_name, args_json),
            None => match leased_hidden_tool_call {
                Some(leased_hidden_tool_call) => leased_hidden_tool_call,
                None => (canonical_tool_name, args_json),
            },
        }
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

fn direct_tool_intent_from_invoke_payload(payload: &Value) -> Option<(String, Value)> {
    let payload_object = payload.as_object()?;
    let tool_id = payload_object.get("tool_id")?.as_str()?;
    let direct_tool_name = tools::direct_tool_name_for_hidden_tool(tool_id)?;
    let arguments = payload_object.get("arguments")?.clone();

    Some((direct_tool_name.to_owned(), arguments))
}

fn discoverable_tool_name(raw_tool_name: &str) -> Option<&'static str> {
    let resolved = tools::resolve_tool_execution(raw_tool_name)?;
    if tools::is_provider_exposed_tool_name(resolved.canonical_name) {
        return None;
    }

    tools::direct_tool_name_for_hidden_tool(resolved.canonical_name)
        .or_else(|| tools::hidden_facade_tool_name_for_hidden_tool(resolved.canonical_name))
        .or(Some(resolved.canonical_name))
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
struct JsonToolBlockParseTelemetry {
    status: &'static str,
    tool_count: usize,
    error_code: Option<&'static str>,
}

impl JsonToolBlockParseTelemetry {
    fn parsed(tool_count: usize) -> Self {
        Self {
            status: "parsed",
            tool_count,
            error_code: None,
        }
    }

    fn malformed(tool_count: usize, error_code: JsonToolBlockParseError) -> Self {
        Self {
            status: "malformed",
            tool_count,
            error_code: Some(error_code.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
enum JsonToolBlockParseResult {
    Parsed {
        cleaned_text: String,
        tool_intents: Vec<ToolIntent>,
        telemetry: JsonToolBlockParseTelemetry,
    },
    Malformed {
        telemetry: JsonToolBlockParseTelemetry,
    },
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonToolBlockParseError {
    MissingToolCallClose,
    InvalidJson,
    UnsupportedShape,
}

impl JsonToolBlockParseError {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingToolCallClose => "missing_tool_call_close",
            Self::InvalidJson => "invalid_json",
            Self::UnsupportedShape => "unsupported_shape",
        }
    }
}

#[derive(Debug, Clone)]
enum JsonToolBlockCandidate {
    Parsed {
        consumed_bytes: usize,
        tool_intent: ToolIntent,
    },
    Malformed(JsonToolBlockParseError),
    Unsupported {
        consumed_bytes: Option<usize>,
    },
}

#[derive(Debug, Clone)]
struct JsonToolCallEnvelope {
    raw_tool_name: String,
    args_json: Value,
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonToolCallEnvelopeMode {
    PlainStandalone,
    TaggedBlock,
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

fn attach_inline_function_parse_telemetry(
    raw_meta: &mut Value,
    telemetry: InlineFunctionParseTelemetry,
) {
    attach_provider_parse_telemetry(
        raw_meta,
        "inline_function",
        telemetry.status,
        telemetry.tool_count,
        telemetry.error_code,
    );
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

fn attach_json_tool_block_parse_telemetry(
    raw_meta: &mut Value,
    telemetry: JsonToolBlockParseTelemetry,
) {
    attach_provider_parse_telemetry(
        raw_meta,
        "json_tool_block",
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

fn extract_json_tool_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> JsonToolBlockParseResult {
    match extract_tagged_json_tool_call_turn(text, session_id, turn_id, bridge_context) {
        JsonToolBlockParseResult::Absent => {
            extract_plain_json_tool_call_turn(text, session_id, turn_id, bridge_context)
        }
        result @ JsonToolBlockParseResult::Parsed { .. }
        | result @ JsonToolBlockParseResult::Malformed { .. } => result,
    }
}

fn extract_tagged_json_tool_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> JsonToolBlockParseResult {
    const TOOL_CALL_OPEN: &str = "<tool_call>";
    const TOOL_CALL_CLOSE: &str = "</tool_call>";

    let mut cursor = 0usize;
    let mut cleaned = String::new();
    let mut tool_intents = Vec::new();
    let mut found_json_tool_block = false;

    while let Some(relative_start) = text[cursor..].find(TOOL_CALL_OPEN) {
        let start = cursor + relative_start;
        if !is_standalone_block_start(text, start)
            || is_inside_markdown_fence(text, start)
            || is_inside_markdown_indented_code_block(text, start)
        {
            let next_cursor = start + TOOL_CALL_OPEN.len();
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
        }

        let body_start = start + TOOL_CALL_OPEN.len();
        let body_remainder = &text[body_start..];
        let Some(body_end) = body_remainder.find(TOOL_CALL_CLOSE) else {
            return JsonToolBlockParseResult::Malformed {
                telemetry: JsonToolBlockParseTelemetry::malformed(
                    tool_intents.len(),
                    JsonToolBlockParseError::MissingToolCallClose,
                ),
            };
        };
        let block_end = body_start + body_end + TOOL_CALL_CLOSE.len();
        if !is_standalone_block_end(text, block_end) {
            cleaned.push_str(&text[cursor..block_end]);
            cursor = block_end;
            continue;
        }

        let block_body = &text[body_start..body_start + body_end];
        let parsed_tool_intents = match parse_json_tool_call_sequence(
            block_body,
            session_id,
            turn_id,
            bridge_context,
            tool_intents.len(),
        ) {
            Ok(parsed_tool_intents) => parsed_tool_intents,
            Err(JsonToolBlockParseError::InvalidJson) => {
                let fallback_tool_intents = parse_wrapped_non_json_tool_calls(
                    block_body,
                    session_id,
                    turn_id,
                    bridge_context,
                    tool_intents.len(),
                );
                let Some(fallback_tool_intents) = fallback_tool_intents else {
                    return JsonToolBlockParseResult::Malformed {
                        telemetry: JsonToolBlockParseTelemetry::malformed(
                            tool_intents.len(),
                            JsonToolBlockParseError::InvalidJson,
                        ),
                    };
                };
                fallback_tool_intents
            }
            Err(error_code) => {
                return JsonToolBlockParseResult::Malformed {
                    telemetry: JsonToolBlockParseTelemetry::malformed(
                        tool_intents.len(),
                        error_code,
                    ),
                };
            }
        };

        found_json_tool_block = true;
        cleaned.push_str(&text[cursor..start]);
        tool_intents.extend(parsed_tool_intents);
        cursor = block_end;
    }

    if !found_json_tool_block {
        return JsonToolBlockParseResult::Absent;
    }

    cleaned.push_str(&text[cursor..]);
    JsonToolBlockParseResult::Parsed {
        cleaned_text: normalize_text(cleaned.as_str()).unwrap_or_default(),
        telemetry: JsonToolBlockParseTelemetry::parsed(tool_intents.len()),
        tool_intents,
    }
}

fn extract_plain_json_tool_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> JsonToolBlockParseResult {
    let mut cursor = 0usize;
    let mut cleaned = String::new();
    let mut tool_intents = Vec::new();
    let mut found_json_tool_block = false;

    while let Some(relative_start) = text[cursor..].find('{') {
        let start = cursor + relative_start;
        if !is_standalone_block_start(text, start)
            || is_inside_markdown_fence(text, start)
            || is_inside_markdown_indented_code_block(text, start)
        {
            let next_cursor = start + 1;
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
        }

        match parse_plain_json_tool_call_candidate(
            &text[start..],
            session_id,
            turn_id,
            bridge_context,
            tool_intents.len(),
        ) {
            JsonToolBlockCandidate::Parsed {
                consumed_bytes,
                tool_intent,
            } => {
                found_json_tool_block = true;
                cleaned.push_str(&text[cursor..start]);
                tool_intents.push(tool_intent);
                cursor = start + consumed_bytes;
            }
            JsonToolBlockCandidate::Malformed(error_code) => {
                return JsonToolBlockParseResult::Malformed {
                    telemetry: JsonToolBlockParseTelemetry::malformed(
                        tool_intents.len(),
                        error_code,
                    ),
                };
            }
            JsonToolBlockCandidate::Unsupported { consumed_bytes } => {
                let next_cursor = consumed_bytes
                    .map(|consumed_bytes| start + consumed_bytes)
                    .unwrap_or(start + 1);
                cleaned.push_str(&text[cursor..next_cursor]);
                cursor = next_cursor;
            }
        }
    }

    if !found_json_tool_block {
        return JsonToolBlockParseResult::Absent;
    }

    cleaned.push_str(&text[cursor..]);
    JsonToolBlockParseResult::Parsed {
        cleaned_text: normalize_text(cleaned.as_str()).unwrap_or_default(),
        telemetry: JsonToolBlockParseTelemetry::parsed(tool_intents.len()),
        tool_intents,
    }
}

fn parse_json_tool_call_sequence(
    body: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_offset: usize,
) -> Result<Vec<ToolIntent>, JsonToolBlockParseError> {
    let stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
    let mut tool_intents = Vec::new();

    for result in stream {
        let value = result.map_err(|_error| JsonToolBlockParseError::InvalidJson)?;
        let envelope = json_tool_call_envelope(&value, JsonToolCallEnvelopeMode::TaggedBlock)?
            .ok_or(JsonToolBlockParseError::UnsupportedShape)?;
        tool_intents.push(build_json_tool_intent(
            envelope,
            session_id,
            turn_id,
            bridge_context,
            tool_offset + tool_intents.len(),
        ));
    }

    if tool_intents.is_empty() {
        return Err(JsonToolBlockParseError::InvalidJson);
    }

    Ok(tool_intents)
}

fn parse_wrapped_non_json_tool_calls(
    block_body: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_offset: usize,
) -> Option<Vec<ToolIntent>> {
    let mut nested_raw_meta = Value::Object(serde_json::Map::new());
    let extraction = extract_openai_text_tool_turn(
        block_body,
        &mut nested_raw_meta,
        session_id,
        turn_id,
        bridge_context,
    );
    let has_tool_intents = !extraction.tool_intents.is_empty();
    let has_residual_text = !extraction.assistant_text.is_empty();
    if !has_tool_intents || has_residual_text {
        return None;
    }

    let mut wrapped_tool_intents = extraction.tool_intents;
    for (index, intent) in wrapped_tool_intents.iter_mut().enumerate() {
        let wrapped_tool_call_id = format!("wrapped-call-{}", tool_offset + index);
        intent.tool_call_id = wrapped_tool_call_id;
    }

    Some(wrapped_tool_intents)
}

fn parse_plain_json_tool_call_candidate(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_offset: usize,
) -> JsonToolBlockCandidate {
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let Some(result) = stream.next() else {
        return JsonToolBlockCandidate::Malformed(JsonToolBlockParseError::InvalidJson);
    };
    let value = match result {
        Ok(value) => value,
        Err(_) => return JsonToolBlockCandidate::Malformed(JsonToolBlockParseError::InvalidJson),
    };
    let consumed_bytes = stream.byte_offset();
    if !is_standalone_block_end(text, consumed_bytes) {
        return JsonToolBlockCandidate::Unsupported {
            consumed_bytes: None,
        };
    }
    let envelope = match json_tool_call_envelope(&value, JsonToolCallEnvelopeMode::PlainStandalone)
    {
        Ok(Some(envelope)) => envelope,
        Ok(None) => {
            return JsonToolBlockCandidate::Unsupported {
                consumed_bytes: Some(consumed_bytes),
            };
        }
        Err(error) => return JsonToolBlockCandidate::Malformed(error),
    };
    JsonToolBlockCandidate::Parsed {
        consumed_bytes,
        tool_intent: build_json_tool_intent(
            envelope,
            session_id,
            turn_id,
            bridge_context,
            tool_offset,
        ),
    }
}

fn json_tool_call_envelope(
    value: &Value,
    mode: JsonToolCallEnvelopeMode,
) -> Result<Option<JsonToolCallEnvelope>, JsonToolBlockParseError> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let function = object.get("function").and_then(Value::as_object);
    let Some(raw_tool_name) = object
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| object.get("tool").and_then(Value::as_str))
        .or_else(|| object.get("tool_name").and_then(Value::as_str))
        .or_else(|| {
            function
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
        })
    else {
        return Ok(None);
    };

    let args_json = if let Some(arguments) = json_tool_argument_value(object, function) {
        parse_json_tool_arguments_value(arguments)?
    } else if matches!(mode, JsonToolCallEnvelopeMode::TaggedBlock)
        || has_explicit_json_tool_call_marker(object)
    {
        json_tool_arguments_from_top_level(object)
    } else {
        return Ok(None);
    };

    let tool_call_id = object
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| object.get("tool_call_id").and_then(Value::as_str))
        .or_else(|| object.get("call_id").and_then(Value::as_str))
        .or_else(|| {
            function
                .and_then(|function| function.get("id"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned);

    Ok(Some(JsonToolCallEnvelope {
        raw_tool_name: raw_tool_name.to_owned(),
        args_json,
        tool_call_id,
    }))
}

fn build_json_tool_intent(
    envelope: JsonToolCallEnvelope,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_index: usize,
) -> ToolIntent {
    build_provider_tool_intent(
        envelope.raw_tool_name.as_str(),
        envelope.args_json,
        "provider_json_tool_call",
        session_id,
        turn_id,
        envelope
            .tool_call_id
            .unwrap_or_else(|| format!("json-call-{tool_index}")),
        bridge_context,
    )
}

fn json_tool_argument_value<'a>(
    object: &'a serde_json::Map<String, Value>,
    function: Option<&'a serde_json::Map<String, Value>>,
) -> Option<&'a Value> {
    object
        .get("arguments")
        .or_else(|| object.get("input"))
        .or_else(|| object.get("parameters"))
        .or_else(|| object.get("args"))
        .or_else(|| object.get("payload"))
        .or_else(|| function.and_then(|function| function.get("arguments")))
        .or_else(|| function.and_then(|function| function.get("input")))
        .or_else(|| function.and_then(|function| function.get("parameters")))
}

fn has_explicit_json_tool_call_marker(object: &serde_json::Map<String, Value>) -> bool {
    object.contains_key("arguments")
        || object.contains_key("input")
        || object.contains_key("parameters")
        || object.contains_key("args")
        || object.contains_key("payload")
        || object.contains_key("function")
        || object.contains_key("type")
}

fn parse_json_tool_arguments_value(value: &Value) -> Result<Value, JsonToolBlockParseError> {
    match value {
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .map_err(|_error| JsonToolBlockParseError::InvalidJson),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Ok(value.clone())
        }
    }
}

fn json_tool_arguments_from_top_level(object: &serde_json::Map<String, Value>) -> Value {
    const RESERVED_FIELDS: &[&str] = &[
        "name",
        "tool",
        "tool_name",
        "function",
        "id",
        "tool_call_id",
        "call_id",
        "type",
        "arguments",
        "input",
        "parameters",
        "args",
        "payload",
    ];

    let mut payload = serde_json::Map::new();
    for (key, value) in object {
        if RESERVED_FIELDS.contains(&key.as_str()) {
            continue;
        }
        payload.insert(key.clone(), value.clone());
    }
    Value::Object(payload)
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
        let args_json = parse_invoke_arguments(canonical_tool_name.as_str(), raw_arguments.trim())?;
        let tool_call_id = format!("invoke-call-{}", tool_call_offset + tool_intents.len());
        tool_intents.push(build_provider_tool_intent(
            canonical_tool_name.as_str(),
            args_json,
            "provider_invoke_block_call",
            session_id,
            turn_id,
            tool_call_id,
            bridge_context,
        ));

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

fn parse_invoke_arguments(
    canonical_tool_name: &str,
    raw_arguments: &str,
) -> Result<Value, InvokeBlockParseError> {
    let decoded = decode_inline_xml_text(raw_arguments);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }

    let parsed = serde_json::from_str::<Value>(trimmed);
    if let Ok(value) = parsed {
        return normalize_invoke_arguments_value(canonical_tool_name, value);
    }

    let backslash_unescaped = decode_backslash_escaped_quotes(trimmed);
    let reparsed = serde_json::from_str::<Value>(backslash_unescaped.as_str());
    if let Ok(value) = reparsed {
        return normalize_invoke_arguments_value(canonical_tool_name, value);
    }

    if canonical_tool_name == "tool.search" {
        return Ok(json!({ "query": trimmed }));
    }

    Err(InvokeBlockParseError::InvalidArgumentsJson)
}

fn normalize_invoke_arguments_value(
    canonical_tool_name: &str,
    value: Value,
) -> Result<Value, InvokeBlockParseError> {
    match value {
        Value::String(query) if canonical_tool_name == "tool.search" => {
            Ok(json!({ "query": query }))
        }
        other @ Value::Null
        | other @ Value::Bool(_)
        | other @ Value::Number(_)
        | other @ Value::String(_)
        | other @ Value::Array(_)
        | other @ Value::Object(_) => Ok(other),
    }
}

fn decode_backslash_escaped_quotes(raw: &str) -> String {
    let single_quotes_unescaped = raw.replace("\\'", "'");
    single_quotes_unescaped.replace("\\\"", "\"")
}

fn extract_inline_function_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> InlineFunctionParseResult {
    match extract_xml_inline_function_call_turn(text, session_id, turn_id, bridge_context) {
        InlineFunctionParseResult::Absent => {
            extract_bracket_inline_function_call_turn(text, session_id, turn_id, bridge_context)
        }
        result @ InlineFunctionParseResult::Parsed { .. }
        | result @ InlineFunctionParseResult::Malformed { .. } => result,
    }
}

fn extract_xml_inline_function_call_turn(
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

fn extract_bracket_inline_function_call_turn(
    text: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> InlineFunctionParseResult {
    const FUNCTION_OPEN: &str = "[";
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
        let Some(header_end) = header_remainder.find(']') else {
            let next_cursor = start + FUNCTION_OPEN.len();
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
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
        let body_trimmed = body_remainder.trim_start_matches([' ', '\t', '\r', '\n']);
        if !body_trimmed.starts_with("<parameter=") {
            let next_cursor = body_start;
            cleaned.push_str(&text[cursor..next_cursor]);
            cursor = next_cursor;
            continue;
        }

        let skipped_body_whitespace = body_remainder.len() - body_trimmed.len();
        let parameter_start = body_start + skipped_body_whitespace;
        let parameter_remainder = &text[parameter_start..];
        let Some(body_end) = parameter_remainder.find(FUNCTION_CLOSE) else {
            return InlineFunctionParseResult::Malformed {
                telemetry: InlineFunctionParseTelemetry::malformed(
                    tool_intents.len(),
                    InlineFunctionParseError::MissingFunctionClose,
                ),
            };
        };

        let function_body = &parameter_remainder[..body_end];
        let function_end = parameter_start + body_end + FUNCTION_CLOSE.len();
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

fn is_standalone_inline_function_start(text: &str, start: usize) -> bool {
    is_standalone_block_start(text, start)
}

fn is_standalone_inline_function_end(text: &str, end: usize) -> bool {
    is_standalone_block_end(text, end)
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

    fn discovery_followup_part_messages(tool_id: &str, lease: &str) -> Vec<Value> {
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
            "content": [{
                "type": "input_text",
                "text": format!("[tool_result]\n[ok] {envelope}"),
            }],
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
        assert_eq!(turn.tool_intents[0].tool_name, "read");
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
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
    }

    #[test]
    fn extract_provider_turn_with_scope_prefers_direct_surface_for_direct_tools_after_search() {
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
        let messages = discovery_followup_messages("read", "lease-openai");

        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-shape"),
            Some("turn-shape"),
            &messages,
        )
        .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].session_id, "session-shape");
        assert_eq!(turn.tool_intents[0].turn_id, "turn-shape");
        assert_eq!(turn.tool_intents[0].tool_call_id, "call_compat");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path":"README.md"}));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn extract_provider_turn_with_scope_rewrites_runtime_discovered_feishu_tools_to_tool_invoke_after_search()
     {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "updating card",
                    "tool_calls": [{
                        "id": "call_feishu_card_update_1",
                        "type": "function",
                        "function": {
                            "name": "feishu_card_update",
                            "arguments": "{\"markdown\":\"callback updated\"}"
                        }
                    }]
                }
            }]
        });
        let messages = discovery_followup_messages("feishu.card.update", "lease-feishu");

        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-feishu"),
            Some("turn-feishu"),
            &messages,
        )
        .expect("turn");
        assert_eq!(turn.assistant_text, "updating card");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
        assert_eq!(turn.tool_intents[0].session_id, "session-feishu");
        assert_eq!(turn.tool_intents[0].turn_id, "turn-feishu");
        assert_eq!(
            turn.tool_intents[0].tool_call_id,
            "call_feishu_card_update_1"
        );
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "channel");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"markdown":"callback updated","operation":"card.update"})
        );
        assert_eq!(turn.tool_intents[0].args_json["lease"], "lease-feishu");
    }

    #[test]
    fn bridge_context_skips_truncated_search_results() {
        let payload_summary = serde_json::to_string(&json!({
            "results": [
                {
                    "tool_id": "read",
                    "lease": "lease-truncated",
                }
            ]
        }))
        .expect("encode");
        let envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": payload_summary.chars().count(),
            "payload_truncated": true,
        }))
        .expect("encode envelope");
        let messages = vec![json!({
            "role": "assistant",
            "content": format!("[tool_result]\n[ok] {envelope}"),
        })];

        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "reading",
                    "tool_calls": [{
                        "id": "call_trunc",
                        "type": "function",
                        "function": {
                            "name": "file_read",
                            "arguments": "{\"path\":\"README.md\"}"
                        }
                    }]
                }
            }]
        });
        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-trunc"),
            Some("turn-trunc"),
            &messages,
        )
        .expect("turn");
        // When payload is truncated, bridge context should be empty,
        // so the hidden alias should fall back to the direct visible surface instead of tool.invoke.
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    }

    #[test]
    fn bridge_context_ignores_compacted_direct_surface_leases() {
        let payload_summary = serde_json::to_string(&json!({
            "query": "read repo file",
            "results": [
                {
                    "tool_id": "read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "lease": "lease-compacted"
                }
            ]
        }))
        .expect("encode compacted search payload summary");
        let envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": payload_summary.chars().count(),
            "payload_truncated": false,
        }))
        .expect("encode search envelope");
        let messages = vec![json!({
            "role": "assistant",
            "content": format!("[tool_result]\n[ok] {envelope}"),
        })];

        let context = provider_tool_bridge_context_from_messages(&messages);
        assert!(
            context.discoverable_leases.is_empty(),
            "direct surfaces should stay direct instead of being rebound through lease context"
        );
    }

    #[test]
    fn provider_shape_discovery_followup_keeps_direct_surfaces_out_of_lease_context_even_with_multiple_results()
     {
        let first_summary = serde_json::to_string(&json!({
            "query": "read repo file",
            "results": [
                {
                    "tool_id": "read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "lease": "lease-first"
                }
            ]
        }))
        .expect("encode first search payload summary");
        let second_summary = serde_json::to_string(&json!({
            "query": "read repo file again",
            "results": [
                {
                    "tool_id": "read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "lease": "lease-second"
                }
            ]
        }))
        .expect("encode second search payload summary");
        let first_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-1",
            "payload_summary": first_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode first search envelope");
        let second_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-2",
            "payload_summary": second_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode second search envelope");
        let messages = vec![json!({
            "role": "assistant",
            "content": format!("[tool_result]\n[ok] {first_envelope}\n[ok] {second_envelope}"),
        })];

        let context = provider_tool_bridge_context_from_messages(&messages);
        assert!(
            context.discoverable_leases.is_empty(),
            "direct surfaces should not accumulate lease state from search followups"
        );
    }

    #[test]
    fn provider_shape_discovery_followup_uses_latest_hidden_lease_in_multiline_source_order() {
        let first_summary = serde_json::to_string(&json!({
            "query": "external skills list",
            "results": [
                {
                    "tool_id": "skills",
                    "summary": "Search, inspect, install, list, or run external skills.",
                    "argument_hint": "operation:string",
                    "required_fields": ["operation"],
                    "required_field_groups": [["operation"]],
                    "lease": "lease-first"
                }
            ]
        }))
        .expect("encode first search payload summary");
        let second_summary = serde_json::to_string(&json!({
            "query": "external skills list again",
            "results": [
                {
                    "tool_id": "skills",
                    "summary": "Search, inspect, install, list, or run external skills.",
                    "argument_hint": "operation:string",
                    "required_fields": ["operation"],
                    "required_field_groups": [["operation"]],
                    "lease": "lease-second"
                }
            ]
        }))
        .expect("encode second search payload summary");
        let first_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-1",
            "payload_summary": first_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode first search envelope");
        let second_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-2",
            "payload_summary": second_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode second search envelope");
        let messages = vec![json!({
            "role": "assistant",
            "content": format!("[tool_result]\n[ok] {first_envelope}\n[ok] {second_envelope}"),
        })];

        let context = provider_tool_bridge_context_from_messages(&messages);
        assert_eq!(
            context.discoverable_leases.get("skills"),
            Some(&"lease-second".to_owned())
        );
    }

    #[test]
    fn provider_shape_discovery_followup_ignores_newer_mixed_content_messages() {
        let latest_summary = serde_json::to_string(&json!({
            "query": "external skills list",
            "results": [
                {
                    "tool_id": "skills",
                    "summary": "Search, inspect, install, list, or run external skills.",
                    "argument_hint": "operation:string",
                    "required_fields": ["operation"],
                    "required_field_groups": [["operation"]],
                    "lease": "lease-latest"
                }
            ]
        }))
        .expect("encode latest search payload summary");
        let stale_summary = serde_json::to_string(&json!({
            "query": "older external skills card",
            "results": [
                {
                    "tool_id": "skills",
                    "summary": "Search, inspect, install, list, or run external skills.",
                    "argument_hint": "operation:string",
                    "required_fields": ["operation"],
                    "required_field_groups": [["operation"]],
                    "lease": "lease-stale"
                }
            ]
        }))
        .expect("encode stale search payload summary");
        let latest_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-latest",
            "payload_summary": latest_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode latest search envelope");
        let stale_envelope = serde_json::to_string(&json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search-stale",
            "payload_summary": stale_summary,
            "payload_chars": 0,
            "payload_truncated": false,
        }))
        .expect("encode stale search envelope");
        let messages = vec![
            json!({
                "role": "assistant",
                "content": format!("[tool_result]\n[ok] {latest_envelope}"),
            }),
            json!({
                "role": "assistant",
                "content": format!(
                    "I am quoting an older card for context.\n[tool_result]\n[ok] {stale_envelope}\nPlease refresh before reusing it."
                ),
            }),
        ];

        let context = provider_tool_bridge_context_from_messages(&messages);
        assert_eq!(
            context.discoverable_leases.get("skills"),
            Some(&"lease-latest".to_owned())
        );
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
        let messages = discovery_followup_messages("read", "lease-responses");
        let turn = extract_provider_turn_with_scope(
            &body,
            Some("session-responses"),
            Some("turn-responses"),
        )
        .expect("responses turn without search context should stay direct");
        assert_eq!(turn.assistant_text, "Reading the file.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
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
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
    }

    #[test]
    fn extract_provider_turn_supports_responses_textual_tool_calls() {
        let body = serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Let me run it.\n[tool_invoke]\n<parameter=tool_id>shell.exec</parameter>\n<parameter=lease>lease-responses-text</parameter>\n<parameter=arguments>{\"command\":\"echo\",\"args\":[\"hello\"]}</parameter>\n</function>"
                        }
                    ]
                }
            ]
        });

        let turn = extract_provider_turn_with_scope(
            &body,
            Some("session-responses-text"),
            Some("turn-responses-text"),
        )
        .expect("responses textual tool turn");
        assert_eq!(turn.assistant_text, "Let me run it.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
        assert_eq!(turn.tool_intents[0].session_id, "session-responses-text");
        assert_eq!(turn.tool_intents[0].turn_id, "turn-responses-text");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "echo",
                "args": ["hello"]
            })
        );
    }

    #[test]
    fn extract_provider_turn_supports_responses_function_calls_with_array_followup_messages() {
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
        let messages = discovery_followup_part_messages("file.read", "lease-responses-parts");

        let turn = extract_provider_turn_with_scope_and_messages(
            &body,
            Some("session-responses"),
            Some("turn-responses"),
            &messages,
        )
        .expect("responses turn with array-form search context");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
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
        let messages = discovery_followup_messages("exec", "lease-shell-inline");

        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(
            turn.assistant_text,
            "sorry, that command failed. let me retry with a simpler approach:"
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({"command":"ls /root"})
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
            "parsed"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["inline_function"]["tool_count"],
            1
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
            "parsed"
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
        assert_eq!(turn.tool_intents[0].args_json["tool_id"], "skills");
        assert_eq!(
            turn.tool_intents[0].args_json["arguments"],
            json!({"skill_id":"home-assistant-1-0-0","action":"get_states","operation":"run"})
        );
        assert_eq!(
            turn.tool_intents[0].args_json["lease"],
            "lease-external-skill-inline"
        );
    }

    #[test]
    fn extract_provider_turn_parses_plain_json_tool_block() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me search for the right tool first.\n{\n  \"name\": \"tool_search\",\n  \"arguments\": {\n    \"query\": \"read note.md\",\n    \"limit\": 3\n  }\n}"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(
            turn.assistant_text,
            "let me search for the right tool first."
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.search");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "query": "read note.md",
                "limit": 3
            })
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
            "parsed"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["tool_count"],
            1
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
            "parsed"
        );
    }

    #[test]
    fn extract_provider_turn_parses_tool_call_wrapped_json_blocks() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me search for the right tool first.\n<tool_call>\n{\"name\":\"tool_search\",\"arguments\":{\"query\":\"read note.md\",\"limit\":3}}\n</tool_call>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(
            turn.assistant_text,
            "let me search for the right tool first."
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.search");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "query": "read note.md",
                "limit": 3
            })
        );
    }

    #[test]
    fn extract_provider_turn_parses_tool_call_wrapped_top_level_json_arguments() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me search for the right tool first.\n<tool_call>\n{\"name\":\"tool_search\",\"query\":\"read note.md\",\"limit\":3}\n</tool_call>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(
            turn.assistant_text,
            "let me search for the right tool first."
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.search");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "query": "read note.md",
                "limit": 3
            })
        );
    }

    #[test]
    fn extract_provider_turn_parses_tool_call_wrapped_inline_function_blocks() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me run the command.\n<tool_call>\n<function=tool_invoke><parameter=tool_id>shell.exec</parameter><parameter=lease>lease-inline-wrap</parameter><parameter=arguments>{\"command\":\"echo\",\"args\":[\"hello\"]}</parameter></function>\n</tool_call>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "let me run the command.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "echo",
                "args": ["hello"]
            })
        );
    }

    #[test]
    fn extract_provider_turn_parses_function_calls_invoke_blocks() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me search for the right tool first.\n<function_calls>\n<invoke name=\"tool.search\" arguments=\"{&quot;query&quot;:&quot;read note.md&quot;,&quot;limit&quot;:3}\"></invoke>\n</function_calls>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(
            turn.assistant_text,
            "let me search for the right tool first."
        );
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.search");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "query": "read note.md",
                "limit": 3
            })
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["invoke_block"]["status"],
            "parsed"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["invoke_block"]["status"],
            "parsed"
        );
    }

    #[test]
    fn extract_provider_turn_parses_invoke_blocks_with_quoted_gt_in_arguments() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me run the shell command.\n<function_calls>\n<invoke name=\"shell.exec\" arguments=\"{&quot;command&quot;:&quot;sh&quot;,&quot;args&quot;:[&quot;-lc&quot;,&quot;echo hi > out.txt&quot;]}\"></invoke>\n</function_calls>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");

        assert_eq!(turn.assistant_text, "let me run the shell command.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "sh",
                "args": ["-lc", "echo hi > out.txt"]
            })
        );
    }

    #[test]
    fn extract_provider_turn_parses_invoke_blocks_with_backslash_escaped_quotes() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": r#"let me search the catalog.
<function_calls>
<invoke name="tool.search" arguments="{\"query\":\"a > b\",\"limit\":3}"></invoke>
</function_calls>"#
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");

        assert_eq!(turn.assistant_text, "let me search the catalog.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "tool.search");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "query": "a > b",
                "limit": 3
            })
        );
    }

    #[test]
    fn extract_provider_turn_parses_bracket_inline_function_blocks() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me handle this.\n[tool_invoke]\n<parameter=tool_id>shell.exec</parameter>\n<parameter=lease>lease-bracket-inline</parameter>\n<parameter=arguments>{\"command\":\"echo\",\"args\":[\"hello\"]}</parameter>\n</function>"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert_eq!(turn.assistant_text, "let me handle this.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
        assert_eq!(
            turn.tool_intents[0].args_json,
            json!({
                "command": "echo",
                "args": ["hello"]
            })
        );
    }

    #[test]
    fn extract_provider_turn_prefers_direct_surface_for_function_call_followups_after_search() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "now i'll read the file.\n<function_calls>\n<invoke name=\"file_read\" arguments=\"{&quot;path&quot;:&quot;note.md&quot;}\"></invoke>\n</function_calls>"
                }
            }]
        });
        let messages = discovery_followup_messages("read", "lease-invoke-followup");

        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "now i'll read the file.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
    }

    #[test]
    fn extract_provider_turn_prefers_direct_surface_for_plain_json_followups_after_search() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "now i'll read the file.\n{\n  \"name\": \"file_read\",\n  \"arguments\": {\n    \"path\": \"note.md\"\n  }\n}"
                }
            }]
        });
        let messages = discovery_followup_messages("read", "lease-json-followup");

        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "now i'll read the file.");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "note.md"}));
    }

    #[test]
    fn extract_provider_turn_does_not_execute_plain_json_top_level_arguments_without_envelope() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n{\n  \"name\": \"tool_search\",\n  \"query\": \"read note.md\"\n}"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n{\n  \"name\": \"tool_search\",\n  \"query\": \"read note.md\"\n}"
        );
    }

    #[test]
    fn extract_provider_turn_marks_invalid_stringified_json_tool_arguments_malformed() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "let me search for the right tool first.\n{\n  \"name\": \"tool_search\",\n  \"arguments\": \"{bad\"\n}"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "let me search for the right tool first.\n{\n  \"name\": \"tool_search\",\n  \"arguments\": \"{bad\"\n}"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
            "malformed"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["error_code"],
            "invalid_json"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["json_tool_block"]["status"],
            "malformed"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_nested_tool_like_plain_json_objects() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n{\n  \"meta\":\n  {\n    \"name\": \"tool_search\",\n    \"arguments\": {\n      \"query\": \"read note.md\"\n    }\n  }\n}"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n{\n  \"meta\":\n  {\n    \"name\": \"tool_search\",\n    \"arguments\": {\n      \"query\": \"read note.md\"\n    }\n  }\n}"
        );
    }

    #[test]
    fn extract_provider_turn_does_not_execute_fenced_json_tool_examples() {
        let body = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "example:\n```json\n{\n  \"name\": \"tool_search\",\n  \"arguments\": {\n    \"query\": \"read note.md\"\n  }\n}\n```"
                }
            }]
        });

        let turn = extract_provider_turn(&body).expect("turn");
        assert!(turn.tool_intents.is_empty());
        assert_eq!(
            turn.assistant_text,
            "example:\n```json\n{\n  \"name\": \"tool_search\",\n  \"arguments\": {\n    \"query\": \"read note.md\"\n  }\n}\n```"
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
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
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
        assert_eq!(turn.tool_intents[0].tool_name, "exec");
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
            turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
            "malformed"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["inline_function"]["error_code"],
            "missing_function_close"
        );
        assert_eq!(
            turn.raw_meta["loong_provider_parse"]["inline_function"]["status"],
            "malformed"
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
        let messages = discovery_followup_messages("read", "lease-anthropic");
        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
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
        let messages = discovery_followup_messages("read", "lease-bedrock");
        let turn = extract_provider_turn_with_scope_and_messages(&body, None, None, &messages)
            .expect("turn");
        assert_eq!(turn.assistant_text, "checking");
        assert_eq!(turn.tool_intents.len(), 1);
        assert_eq!(turn.tool_intents[0].tool_name, "read");
        assert_eq!(turn.tool_intents[0].tool_call_id, "toolu_1");
        assert_eq!(turn.tool_intents[0].args_json, json!({"path": "README.md"}));
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
