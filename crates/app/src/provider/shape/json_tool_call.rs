use serde_json::Value;

use crate::conversation::turn_engine::ToolIntent;

use super::{
    OpenAiTextToolTurnExtraction, ProviderToolBridgeContext, attach_provider_parse_telemetry,
    build_provider_tool_intent, extract_openai_text_tool_turn, is_inside_markdown_fence,
    is_inside_markdown_indented_code_block, is_standalone_block_end, is_standalone_block_start,
    normalize_text,
};

pub(super) fn extract_json_tool_call_turn(
    text: &str,
    raw_meta: &mut Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
) -> Option<(String, Vec<ToolIntent>)> {
    match extract_json_tool_call_turn_result(text, session_id, turn_id, bridge_context) {
        JsonToolBlockParseResult::Parsed {
            cleaned_text,
            tool_intents,
            telemetry,
        } => {
            attach_json_tool_block_parse_telemetry(raw_meta, telemetry);
            Some((cleaned_text, tool_intents))
        }
        JsonToolBlockParseResult::Malformed { telemetry } => {
            attach_json_tool_block_parse_telemetry(raw_meta, telemetry);
            None
        }
        JsonToolBlockParseResult::Absent => None,
    }
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
        tool_intents: Vec<ToolIntent>,
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

fn extract_json_tool_call_turn_result(
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
            Err(JsonToolBlockParseError::UnsupportedShape) => {
                cleaned.push_str(&text[cursor..block_end]);
                cursor = block_end;
                continue;
            }
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

        if parsed_tool_intents.is_empty() {
            cleaned.push_str(&text[cursor..block_end]);
            cursor = block_end;
            continue;
        }

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

    while let Some(start) = find_next_plain_json_candidate_start(text, cursor) {
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
                tool_intents: parsed_tool_intents,
            } => {
                found_json_tool_block = true;
                let prefix_end = strip_trailing_tool_wrapper_marker_line_start(text, cursor, start)
                    .unwrap_or(start);
                cleaned.push_str(&text[cursor..prefix_end]);
                tool_intents.extend(parsed_tool_intents);
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

fn find_next_plain_json_candidate_start(text: &str, cursor: usize) -> Option<usize> {
    let remainder = text.get(cursor..)?;
    for (relative_index, character) in remainder.char_indices() {
        let absolute_index = cursor + relative_index;
        match character {
            '{' => return Some(absolute_index),
            '[' if array_candidate_starts_with_object(text, absolute_index) => {
                return Some(absolute_index);
            }
            _ => {}
        }
    }
    None
}

fn array_candidate_starts_with_object(text: &str, start: usize) -> bool {
    let Some(remainder) = text.get(start + 1..) else {
        return false;
    };
    remainder
        .chars()
        .find(|character| !character.is_whitespace())
        .is_some_and(|character| character == '{')
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
        let tool_intent = build_json_tool_intent(
            envelope,
            session_id,
            turn_id,
            bridge_context,
            tool_offset + tool_intents.len(),
        )
        .ok_or(JsonToolBlockParseError::UnsupportedShape)?;
        tool_intents.push(tool_intent);
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
    let OpenAiTextToolTurnExtraction {
        assistant_text,
        tool_intents,
    } = extract_openai_text_tool_turn(
        block_body,
        &mut nested_raw_meta,
        session_id,
        turn_id,
        bridge_context,
    );
    if tool_intents.is_empty() || !assistant_text.is_empty() {
        return None;
    }

    Some(
        tool_intents
            .into_iter()
            .enumerate()
            .map(|(index, mut intent)| {
                intent.tool_call_id = format!("wrapped-call-{}", tool_offset + index);
                intent
            })
            .collect(),
    )
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
    if is_standalone_block_end(text, consumed_bytes) {
        if let Some(candidate) = build_plain_json_tool_call_candidate(
            text,
            &value,
            consumed_bytes,
            session_id,
            turn_id,
            bridge_context,
            tool_offset,
        ) {
            return candidate;
        }
        return JsonToolBlockCandidate::Unsupported {
            consumed_bytes: Some(consumed_bytes),
        };
    }

    if let Some((repaired_value, repaired_consumed_bytes)) =
        repair_misordered_json_tool_call_candidate(text, &value, consumed_bytes)
        && let Some(candidate) = build_plain_json_tool_call_candidate(
            text,
            &repaired_value,
            repaired_consumed_bytes,
            session_id,
            turn_id,
            bridge_context,
            tool_offset,
        )
    {
        return candidate;
    }

    if let Some(candidate) = build_plain_json_tool_call_candidate(
        text,
        &value,
        consumed_bytes,
        session_id,
        turn_id,
        bridge_context,
        tool_offset,
    ) {
        return candidate;
    }

    JsonToolBlockCandidate::Unsupported {
        consumed_bytes: None,
    }
}

fn build_plain_json_tool_call_candidate(
    text: &str,
    value: &Value,
    consumed_bytes: usize,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    bridge_context: &ProviderToolBridgeContext,
    tool_offset: usize,
) -> Option<JsonToolBlockCandidate> {
    let standalone_end = is_standalone_block_end(text, consumed_bytes);
    let recoverable_trailing_text = has_recoverable_trailing_text(text, consumed_bytes);
    if !standalone_end && !recoverable_trailing_text {
        return None;
    }

    if let Some(array) = value.as_array() {
        let mut tool_intents = Vec::new();
        for entry in array {
            let envelope =
                match json_tool_call_envelope(entry, JsonToolCallEnvelopeMode::PlainStandalone) {
                    Ok(Some(envelope)) => envelope,
                    Ok(None) => {
                        return Some(JsonToolBlockCandidate::Unsupported {
                            consumed_bytes: Some(consumed_bytes),
                        });
                    }
                    Err(error) => return Some(JsonToolBlockCandidate::Malformed(error)),
                };
            let tool_intent = match build_json_tool_intent(
                envelope,
                session_id,
                turn_id,
                bridge_context,
                tool_offset + tool_intents.len(),
            ) {
                Some(tool_intent) => tool_intent,
                None => {
                    return Some(JsonToolBlockCandidate::Unsupported {
                        consumed_bytes: Some(consumed_bytes),
                    });
                }
            };
            tool_intents.push(tool_intent);
        }

        if tool_intents.is_empty() {
            return Some(JsonToolBlockCandidate::Unsupported {
                consumed_bytes: Some(consumed_bytes),
            });
        }

        return Some(JsonToolBlockCandidate::Parsed {
            consumed_bytes,
            tool_intents,
        });
    }

    let envelope = match json_tool_call_envelope(value, JsonToolCallEnvelopeMode::PlainStandalone) {
        Ok(Some(envelope)) => envelope,
        Ok(None) => {
            return Some(JsonToolBlockCandidate::Unsupported {
                consumed_bytes: Some(consumed_bytes),
            });
        }
        Err(error) => return Some(JsonToolBlockCandidate::Malformed(error)),
    };
    let tool_intent =
        match build_json_tool_intent(envelope, session_id, turn_id, bridge_context, tool_offset) {
            Some(tool_intent) => tool_intent,
            None => {
                return Some(JsonToolBlockCandidate::Unsupported {
                    consumed_bytes: Some(consumed_bytes),
                });
            }
        };

    Some(JsonToolBlockCandidate::Parsed {
        consumed_bytes,
        tool_intents: vec![tool_intent],
    })
}

fn repair_misordered_json_tool_call_candidate(
    text: &str,
    first_value: &Value,
    consumed_bytes: usize,
) -> Option<(Value, usize)> {
    let Value::Object(_) = first_value else {
        return None;
    };

    let suffix = &text[consumed_bytes..];
    let trimmed_prefix_len = suffix.len().saturating_sub(suffix.trim_start().len());
    let trimmed_suffix = &suffix[trimmed_prefix_len..];
    if !trimmed_suffix.starts_with(',') {
        return None;
    }

    let repaired_tail = format!("{{{}", &trimmed_suffix[1..]);
    let mut tail_stream = serde_json::Deserializer::from_str(&repaired_tail).into_iter::<Value>();
    let tail_value = match tail_stream.next() {
        Some(Ok(value)) => value,
        _ => return None,
    };
    let tail_consumed_bytes = tail_stream.byte_offset();
    if !is_standalone_block_end(repaired_tail.as_str(), tail_consumed_bytes)
        && !has_recoverable_trailing_text(repaired_tail.as_str(), tail_consumed_bytes)
    {
        return None;
    }

    let mut tail_object = tail_value.as_object()?.clone();
    let has_tool_name = tail_object.contains_key("name")
        || tail_object.contains_key("tool")
        || tail_object.contains_key("tool_name")
        || tail_object
            .get("function")
            .and_then(Value::as_object)
            .is_some_and(|function| function.contains_key("name"));
    if !has_tool_name {
        return None;
    }

    if !tail_object.contains_key("arguments")
        && !tail_object.contains_key("request")
        && !tail_object.contains_key("input")
        && !tail_object.contains_key("parameters")
        && !tail_object.contains_key("args")
        && !tail_object.contains_key("payload")
    {
        tail_object.insert("request".to_owned(), first_value.clone());
    }

    let repaired_consumed_bytes = consumed_bytes + trimmed_prefix_len + tail_consumed_bytes;
    Some((Value::Object(tail_object), repaired_consumed_bytes))
}

fn has_recoverable_trailing_text(text: &str, end: usize) -> bool {
    let line_end = text[end..]
        .find('\n')
        .map(|relative| end + relative)
        .unwrap_or(text.len());
    !text[end..line_end].trim().is_empty()
}

fn strip_trailing_tool_wrapper_marker_line_start(
    text: &str,
    cursor: usize,
    start: usize,
) -> Option<usize> {
    let current_prefix = text.get(cursor..start)?;
    for marker in ["[tool_request]", "[tool_failure]"] {
        if let Some(relative_marker_start) = current_prefix.rfind(marker) {
            let marker_end = relative_marker_start + marker.len();
            let trailing_suffix = current_prefix.get(marker_end..)?.trim();
            if trailing_suffix.is_empty() {
                return Some(cursor + relative_marker_start);
            }
        }
    }

    let between = text.get(cursor..start)?.trim();
    if matches!(between, "[tool_request]" | "[tool_failure]") {
        return Some(cursor);
    }

    let current_line_start = text[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    if current_line_start == 0 {
        return None;
    }

    let previous_line_end = current_line_start.saturating_sub(1);
    let previous_line_start = text[..previous_line_end]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    if previous_line_start < cursor {
        return None;
    }

    let previous_line = text[previous_line_start..previous_line_end].trim();
    matches!(previous_line, "[tool_request]" | "[tool_failure]").then_some(previous_line_start)
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
) -> Option<ToolIntent> {
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
        .get("request")
        .or_else(|| object.get("arguments"))
        .or_else(|| object.get("input"))
        .or_else(|| object.get("parameters"))
        .or_else(|| object.get("args"))
        .or_else(|| object.get("payload"))
        .or_else(|| function.and_then(|function| function.get("arguments")))
        .or_else(|| function.and_then(|function| function.get("input")))
        .or_else(|| function.and_then(|function| function.get("parameters")))
}

fn has_explicit_json_tool_call_marker(object: &serde_json::Map<String, Value>) -> bool {
    object.contains_key("request")
        || object.contains_key("arguments")
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
        "request",
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
