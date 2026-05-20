use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::conversation::turn_engine::ToolIntent;
use crate::tools;

use super::{
    ProviderToolBridgeContext, attach_provider_parse_telemetry, build_provider_tool_intent,
    decode_inline_xml_text, is_inside_markdown_fence, is_inside_markdown_indented_code_block,
    is_standalone_block_end, is_standalone_block_start, normalize_text,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InvokeBlockParseTelemetry {
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
pub(super) enum InvokeBlockParseResult {
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

pub(super) fn attach_invoke_block_parse_telemetry(
    raw_meta: &mut Value,
    telemetry: InvokeBlockParseTelemetry,
) {
    attach_provider_parse_telemetry(
        raw_meta,
        "invoke_block",
        telemetry.status,
        telemetry.tool_count,
        telemetry.error_code,
    );
}

pub(super) fn extract_invoke_block_turn(
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
