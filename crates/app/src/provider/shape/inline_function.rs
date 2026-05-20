use std::{collections::BTreeMap, sync::OnceLock};

use serde_json::Value;

use super::{
    ProviderToolBridgeContext, attach_provider_parse_telemetry, build_provider_tool_intent,
    decode_inline_xml_text, is_inside_markdown_fence, is_inside_markdown_indented_code_block,
    is_standalone_block_end, is_standalone_block_start, normalize_text,
};
use crate::conversation::turn_engine::ToolIntent;
use crate::tools;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineFunctionParseTelemetry {
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
pub(super) enum InlineFunctionParseResult {
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

pub(super) fn attach_inline_function_parse_telemetry(
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

pub(super) fn extract_inline_function_call_turn(
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

        let tool_call_id = format!("inline-call-{}", tool_intents.len());
        let tool_intent = build_provider_tool_intent(
            canonical_tool_name.as_str(),
            args_json,
            "provider_inline_function_call",
            session_id,
            turn_id,
            tool_call_id,
            bridge_context,
        );
        if let Some(tool_intent) = tool_intent {
            found_inline_function = true;
            cleaned.push_str(&text[cursor..start]);
            tool_intents.push(tool_intent);
        } else {
            cleaned.push_str(&text[cursor..function_end]);
        }

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

        let tool_call_id = format!("inline-call-{}", tool_intents.len());
        let tool_intent = build_provider_tool_intent(
            canonical_tool_name.as_str(),
            args_json,
            "provider_inline_function_call",
            session_id,
            turn_id,
            tool_call_id,
            bridge_context,
        );
        if let Some(tool_intent) = tool_intent {
            found_inline_function = true;
            cleaned.push_str(&text[cursor..start]);
            tool_intents.push(tool_intent);
        } else {
            cleaned.push_str(&text[cursor..function_end]);
        }

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
    is_standalone_block_start(text, start)
}

fn is_standalone_inline_function_end(text: &str, end: usize) -> bool {
    is_standalone_block_end(text, end)
}
