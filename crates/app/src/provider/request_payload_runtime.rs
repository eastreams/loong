use serde_json::{Value, json};

use crate::config::{LoongClawConfig, ProviderProtocolFamily, ReasoningEffort};

use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{
    CompletionPayloadMode, ProviderCapabilityContract, ReasoningField, TemperatureField,
    TokenLimitField, provider_runtime_contract,
};

const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 4_096;

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn build_completion_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
) -> Value {
    let runtime_contract = provider_runtime_contract(&config.provider);
    let capability_profile =
        ProviderCapabilityProfile::from_provider(&config.provider, runtime_contract);
    let capability = capability_profile.resolve_for_model(model);
    build_completion_request_body_with_capability(config, messages, model, payload_mode, capability)
}

pub(super) fn build_completion_request_body_with_capability(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    capability: ProviderCapabilityContract,
) -> Value {
    match config.provider.kind.protocol_family() {
        ProviderProtocolFamily::AnthropicMessages => {
            build_anthropic_request_body(config, messages, model, payload_mode, false, &[])
        }
        ProviderProtocolFamily::BedrockConverse => {
            build_bedrock_request_body(config, messages, payload_mode, false, &[])
        }
        ProviderProtocolFamily::OpenAiChatCompletions => {
            build_openai_compatible_request_body(config, messages, model, payload_mode, capability)
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn build_turn_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    let runtime_contract = provider_runtime_contract(&config.provider);
    let capability_profile =
        ProviderCapabilityProfile::from_provider(&config.provider, runtime_contract);
    let capability = capability_profile.resolve_for_model(model);
    build_turn_request_body_with_capability(
        config,
        messages,
        model,
        payload_mode,
        capability,
        include_tool_schema,
        tool_definitions,
    )
}

pub(super) fn build_turn_request_body_with_capability(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    capability: ProviderCapabilityContract,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    match config.provider.kind.protocol_family() {
        ProviderProtocolFamily::AnthropicMessages => build_anthropic_request_body(
            config,
            messages,
            model,
            payload_mode,
            include_tool_schema,
            tool_definitions,
        ),
        ProviderProtocolFamily::BedrockConverse => build_bedrock_request_body(
            config,
            messages,
            payload_mode,
            include_tool_schema,
            tool_definitions,
        ),
        ProviderProtocolFamily::OpenAiChatCompletions => {
            let mut body = build_openai_compatible_request_body(
                config,
                messages,
                model,
                payload_mode,
                capability,
            );
            if include_tool_schema
                && !tool_definitions.is_empty()
                && let Some(object) = body.as_object_mut()
            {
                object.insert("tools".to_owned(), Value::Array(tool_definitions.to_vec()));
                object.insert("tool_choice".to_owned(), json!("auto"));
            }
            body
        }
    }
}

fn build_openai_compatible_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    capability: ProviderCapabilityContract,
) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(messages.to_vec()));
    body.insert("stream".to_owned(), Value::Bool(false));
    if payload_mode.temperature_field == TemperatureField::Include {
        body.insert("temperature".to_owned(), json!(config.provider.temperature));
    }

    if let Some(limit) = config.provider.max_tokens {
        match payload_mode.token_field {
            TokenLimitField::MaxCompletionTokens => {
                body.insert("max_completion_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::MaxTokens => {
                body.insert("max_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::Omit => {}
        }
    }

    if let Some(reasoning_effort) = config.provider.reasoning_effort {
        match payload_mode.reasoning_field {
            ReasoningField::ReasoningEffort => {
                body.insert(
                    "reasoning_effort".to_owned(),
                    json!(reasoning_effort.as_str()),
                );
            }
            ReasoningField::ReasoningObject => {
                body.insert(
                    "reasoning".to_owned(),
                    json!({
                        "effort": reasoning_effort.as_str()
                    }),
                );
            }
            ReasoningField::Omit => {}
        }
    }

    if capability.include_reasoning_extra_body()
        && let Some(extra_body) = kimi_extra_body(config.provider.reasoning_effort)
    {
        body.insert("extra_body".to_owned(), extra_body);
    }

    Value::Object(body)
}

fn build_anthropic_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    let mut body = serde_json::Map::new();
    let (system, adapted_messages) = adapt_messages_for_anthropic(messages);
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(adapted_messages));
    body.insert("stream".to_owned(), Value::Bool(false));
    body.insert(
        "max_tokens".to_owned(),
        json!(
            config
                .provider
                .max_tokens
                .unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS)
        ),
    );
    if let Some(system) = system {
        body.insert("system".to_owned(), Value::String(system));
    }
    if payload_mode.temperature_field == TemperatureField::Include {
        body.insert("temperature".to_owned(), json!(config.provider.temperature));
    }
    if include_tool_schema {
        let tools = anthropic_tool_definitions(tool_definitions);
        if !tools.is_empty() {
            body.insert("tools".to_owned(), Value::Array(tools));
            body.insert("tool_choice".to_owned(), json!({ "type": "auto" }));
        }
    }
    Value::Object(body)
}

fn build_bedrock_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    let mut body = serde_json::Map::new();
    let (system, adapted_messages) = adapt_messages_for_bedrock(messages);
    body.insert("messages".to_owned(), Value::Array(adapted_messages));
    if !system.is_empty() {
        body.insert("system".to_owned(), Value::Array(system));
    }

    let mut inference_config = serde_json::Map::new();
    if payload_mode.temperature_field == TemperatureField::Include {
        inference_config.insert("temperature".to_owned(), json!(config.provider.temperature));
    }
    if let Some(limit) = config.provider.max_tokens
        && payload_mode.token_field != TokenLimitField::Omit
    {
        inference_config.insert("maxTokens".to_owned(), json!(limit));
    }
    if !inference_config.is_empty() {
        body.insert(
            "inferenceConfig".to_owned(),
            Value::Object(inference_config),
        );
    }

    if include_tool_schema {
        let tools = bedrock_tool_definitions(tool_definitions);
        if !tools.is_empty() {
            body.insert(
                "toolConfig".to_owned(),
                json!({
                    "tools": tools,
                    "toolChoice": {
                        "auto": {}
                    }
                }),
            );
        }
    }

    Value::Object(body)
}

fn adapt_messages_for_anthropic(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut system_parts = Vec::new();
    let mut adapted = Vec::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = message.get("content").unwrap_or(&Value::Null);
        match role {
            "system" => {
                if let Some(text) = anthropic_blocks_as_text(&anthropic_content_blocks(content)) {
                    system_parts.push(text);
                }
            }
            "user" | "assistant" => {
                append_native_message(&mut adapted, role, anthropic_content_blocks(content));
            }
            "tool" => {
                let Some(text) = content_as_text(content) else {
                    continue;
                };
                append_native_message(
                    &mut adapted,
                    "user",
                    vec![anthropic_text_block(format!("[tool]\n{text}"))],
                );
            }
            _ => {}
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };
    (system, adapted)
}

fn adapt_messages_for_bedrock(messages: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut system = Vec::new();
    let mut adapted = Vec::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = message.get("content").unwrap_or(&Value::Null);
        match role {
            "system" => {
                system.extend(bedrock_content_blocks(content));
            }
            "user" | "assistant" => {
                append_native_message(&mut adapted, role, bedrock_content_blocks(content));
            }
            "tool" => {
                let Some(text) = content_as_text(content) else {
                    continue;
                };
                append_native_message(
                    &mut adapted,
                    "user",
                    vec![bedrock_text_block(format!("[tool]\n{text}"))],
                );
            }
            _ => {}
        }
    }

    (system, adapted)
}

fn append_native_message(adapted: &mut Vec<Value>, role: &str, mut blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = adapted.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
    {
        content.append(&mut blocks);
        return;
    }
    adapted.push(json!({
        "role": role,
        "content": Value::Array(blocks),
    }));
}

fn anthropic_content_blocks(content: &Value) -> Vec<Value> {
    if let Some(text) = content.as_str().and_then(normalize_text) {
        return vec![anthropic_text_block(text)];
    }

    if let Some(items) = content.as_array() {
        return items.iter().filter_map(anthropic_content_block).collect();
    }

    if content.is_null() {
        return Vec::new();
    }

    normalize_text(content.to_string().as_str())
        .map(|text| vec![anthropic_text_block(text)])
        .unwrap_or_default()
}

fn bedrock_content_blocks(content: &Value) -> Vec<Value> {
    if let Some(text) = content.as_str().and_then(normalize_text) {
        return vec![bedrock_text_block(text)];
    }

    if let Some(items) = content.as_array() {
        return items.iter().filter_map(bedrock_content_block).collect();
    }

    if content.is_null() {
        return Vec::new();
    }

    normalize_text(content.to_string().as_str())
        .map(|text| vec![bedrock_text_block(text)])
        .unwrap_or_default()
}

fn anthropic_content_block(value: &Value) -> Option<Value> {
    if let Some(text) = value.as_str().and_then(normalize_text) {
        return Some(anthropic_text_block(text));
    }

    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        match kind {
            "text" => {
                if let Some(text) = value.get("text").and_then(content_text_value) {
                    return Some(anthropic_text_block(text));
                }
            }
            "tool_use" | "tool_result" => return Some(value.clone()),
            _ => {}
        }
    }

    if let Some(text) = value.get("text").and_then(content_text_value) {
        return Some(anthropic_text_block(text));
    }

    None
}

fn bedrock_content_block(value: &Value) -> Option<Value> {
    if let Some(text) = value.as_str().and_then(normalize_text) {
        return Some(bedrock_text_block(text));
    }

    if value.get("toolUse").is_some() || value.get("toolResult").is_some() {
        return Some(value.clone());
    }

    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        match kind {
            "text" => {
                if let Some(text) = value.get("text").and_then(content_text_value) {
                    return Some(bedrock_text_block(text));
                }
            }
            "tool_use" => {
                let id = value
                    .get("id")
                    .or_else(|| value.get("tool_use_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if id.is_empty() || name.is_empty() {
                    return None;
                }
                return Some(json!({
                    "toolUse": {
                        "toolUseId": id,
                        "name": name,
                        "input": value.get("input").cloned().unwrap_or_else(|| json!({}))
                    }
                }));
            }
            "tool_result" => {
                let tool_use_id = value
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if tool_use_id.is_empty() {
                    return None;
                }
                let rendered = value
                    .get("content")
                    .and_then(content_text_value)
                    .unwrap_or_default();
                let status = if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                    "error"
                } else {
                    "success"
                };
                return Some(json!({
                    "toolResult": {
                        "toolUseId": tool_use_id,
                        "content": [
                            {
                                "text": rendered
                            }
                        ],
                        "status": status
                    }
                }));
            }
            _ => {}
        }
    }

    if let Some(text) = value.get("text").and_then(content_text_value) {
        return Some(bedrock_text_block(text));
    }

    None
}

fn anthropic_blocks_as_text(blocks: &[Value]) -> Option<String> {
    let mut merged = Vec::new();
    for block in blocks {
        let Some(text) = block.get("text").and_then(content_text_value) else {
            continue;
        };
        merged.push(text);
    }
    if merged.is_empty() {
        return None;
    }
    Some(merged.join("\n"))
}

fn anthropic_text_block(text: String) -> Value {
    json!({
        "type": "text",
        "text": text,
    })
}

fn bedrock_text_block(text: String) -> Value {
    json!({
        "text": text,
    })
}

fn content_as_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str().and_then(normalize_text) {
        return Some(text);
    }
    let parts = anthropic_content_blocks(content);
    anthropic_blocks_as_text(&parts)
}

fn content_text_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().and_then(normalize_text) {
        return Some(text);
    }
    value
        .get("value")
        .and_then(Value::as_str)
        .and_then(normalize_text)
}

fn normalize_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn anthropic_tool_definitions(tool_definitions: &[Value]) -> Vec<Value> {
    tool_definitions
        .iter()
        .filter_map(openai_tool_definition_to_anthropic)
        .collect()
}

fn bedrock_tool_definitions(tool_definitions: &[Value]) -> Vec<Value> {
    tool_definitions
        .iter()
        .filter_map(openai_tool_definition_to_bedrock)
        .collect()
}

fn openai_tool_definition_to_anthropic(tool_definition: &Value) -> Option<Value> {
    let function = tool_definition.get("function")?;
    let name = function.get("name")?.as_str()?;
    let description = function
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input_schema = function.get("parameters").cloned().unwrap_or_else(|| {
        json!({
            "type": "object",
            "properties": {},
        })
    });
    Some(json!({
        "name": name,
        "description": description,
        "input_schema": input_schema,
    }))
}

fn openai_tool_definition_to_bedrock(tool_definition: &Value) -> Option<Value> {
    let function = tool_definition.get("function")?;
    let name = function.get("name")?.as_str()?;
    let description = function
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let input_schema = function.get("parameters").cloned().unwrap_or_else(|| {
        json!({
            "type": "object",
            "properties": {},
        })
    });
    Some(json!({
        "toolSpec": {
            "name": name,
            "description": description,
            "inputSchema": {
                "json": input_schema
            }
        }
    }))
}

fn kimi_extra_body(reasoning_effort: Option<ReasoningEffort>) -> Option<Value> {
    let reasoning_effort = reasoning_effort?;
    let thinking_type = match reasoning_effort {
        ReasoningEffort::None => "disabled",
        ReasoningEffort::Minimal
        | ReasoningEffort::Low
        | ReasoningEffort::Medium
        | ReasoningEffort::High
        | ReasoningEffort::Xhigh => "enabled",
    };
    Some(json!({
        "thinking": {
            "type": thinking_type
        }
    }))
}
