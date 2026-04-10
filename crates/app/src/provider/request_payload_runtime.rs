use serde_json::{Value, json};

use crate::config::{LoongClawConfig, ReasoningEffort};

use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{
    CompletionPayloadMode, ProviderCapabilityContract, ProviderRuntimeContract,
    ProviderTransportMode, ReasoningField, TemperatureField, TokenLimitField,
    provider_runtime_contract,
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
    build_completion_request_body_with_capability(
        config,
        messages,
        model,
        payload_mode,
        runtime_contract,
        capability,
    )
}

pub(super) fn build_completion_request_body_with_capability(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> Value {
    match runtime_contract.transport_mode {
        ProviderTransportMode::Responses => {
            build_responses_request_body(config, messages, model, payload_mode, false, &[], false)
        }
        ProviderTransportMode::AnthropicMessages => {
            build_anthropic_request_body(config, messages, model, payload_mode, false, &[], false)
        }
        ProviderTransportMode::BedrockConverse => {
            build_bedrock_request_body(config, messages, payload_mode, false, &[])
        }
        ProviderTransportMode::GoogleGenerateContent => {
            build_google_generate_content_request_body(config, messages, payload_mode, false, &[])
        }
        ProviderTransportMode::OpenAiChatCompletions | ProviderTransportMode::KimiApi => {
            build_chat_completions_request_body(
                config,
                messages,
                model,
                payload_mode,
                capability,
                false,
            )
        }
    }
}

fn build_chat_completions_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    capability: ProviderCapabilityContract,
    streaming: bool,
) -> Value {
    build_openai_compatible_request_body(
        config,
        messages,
        model,
        payload_mode,
        capability,
        streaming,
    )
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
        runtime_contract,
        capability,
        include_tool_schema,
        tool_definitions,
        false,
    )
}

pub(super) fn build_turn_request_body_with_capability(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
    include_tool_schema: bool,
    tool_definitions: &[Value],
    streaming: bool,
) -> Value {
    match runtime_contract.transport_mode {
        ProviderTransportMode::Responses => build_responses_request_body(
            config,
            messages,
            model,
            payload_mode,
            include_tool_schema,
            tool_definitions,
            streaming,
        ),
        ProviderTransportMode::AnthropicMessages => build_anthropic_request_body(
            config,
            messages,
            model,
            payload_mode,
            include_tool_schema,
            tool_definitions,
            streaming,
        ),
        ProviderTransportMode::BedrockConverse => build_bedrock_request_body(
            config,
            messages,
            payload_mode,
            include_tool_schema,
            tool_definitions,
        ),
        ProviderTransportMode::GoogleGenerateContent => build_google_generate_content_request_body(
            config,
            messages,
            payload_mode,
            include_tool_schema,
            tool_definitions,
        ),
        ProviderTransportMode::OpenAiChatCompletions | ProviderTransportMode::KimiApi => {
            let mut body = build_openai_compatible_request_body(
                config,
                messages,
                model,
                payload_mode,
                capability,
                streaming,
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
    streaming: bool,
) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(messages.to_vec()));
    body.insert("stream".to_owned(), Value::Bool(streaming));
    apply_common_payload_fields(&mut body, config, payload_mode, capability);

    Value::Object(body)
}

fn build_anthropic_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
    streaming: bool,
) -> Value {
    let mut body = serde_json::Map::new();
    let (system, adapted_messages) = adapt_messages_for_anthropic(messages);
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(adapted_messages));
    body.insert("stream".to_owned(), Value::Bool(streaming));
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

fn build_google_generate_content_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    let mut body = serde_json::Map::new();
    let (system_instruction, contents) = adapt_messages_for_google(messages);
    body.insert("contents".to_owned(), Value::Array(contents));

    if let Some(system_instruction) = system_instruction {
        body.insert(
            "systemInstruction".to_owned(),
            json!({
                "parts": [
                    {
                        "text": system_instruction
                    }
                ]
            }),
        );
    }

    let mut generation_config = serde_json::Map::new();
    if payload_mode.temperature_field == TemperatureField::Include {
        generation_config.insert("temperature".to_owned(), json!(config.provider.temperature));
    }
    if let Some(limit) = config.provider.max_tokens
        && payload_mode.token_field != TokenLimitField::Omit
    {
        generation_config.insert("maxOutputTokens".to_owned(), json!(limit));
    }
    if !generation_config.is_empty() {
        body.insert(
            "generationConfig".to_owned(),
            Value::Object(generation_config),
        );
    }

    if include_tool_schema {
        let function_declarations = google_tool_declarations(tool_definitions);
        if !function_declarations.is_empty() {
            body.insert(
                "tools".to_owned(),
                json!([
                    {
                        "functionDeclarations": function_declarations
                    }
                ]),
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

fn adapt_messages_for_google(messages: &[Value]) -> (Option<String>, Vec<Value>) {
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
                if let Some(text) = content_as_text(content) {
                    system_parts.push(text);
                }
            }
            "user" => {
                append_google_message(
                    &mut adapted,
                    "user",
                    google_message_parts(message, messages),
                );
            }
            "assistant" => {
                append_google_message(
                    &mut adapted,
                    "model",
                    google_message_parts(message, messages),
                );
            }
            "tool" => {
                let tool_call_id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let tool_name =
                    resolve_google_tool_name_for_result(messages, tool_call_id).unwrap_or("tool");
                let response = google_tool_response_payload(content);
                append_google_message(
                    &mut adapted,
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "name": tool_name,
                            "response": response,
                        }
                    })],
                );
            }
            _ => {}
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system_instruction, adapted)
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

fn append_google_message(adapted: &mut Vec<Value>, role: &str, mut parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = adapted.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(existing_parts) = last.get_mut("parts").and_then(Value::as_array_mut)
    {
        existing_parts.append(&mut parts);
        return;
    }
    adapted.push(json!({
        "role": role,
        "parts": Value::Array(parts),
    }));
}

fn google_message_parts(message: &Value, messages: &[Value]) -> Vec<Value> {
    let content = message.get("content").unwrap_or(&Value::Null);
    let mut parts = google_content_parts(content, messages);

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tool_call in tool_calls {
            if let Some(function_call) = google_function_call_part_from_openai(tool_call) {
                parts.push(function_call);
            }
        }
    }

    parts
}

fn google_content_parts(content: &Value, messages: &[Value]) -> Vec<Value> {
    if let Some(text) = content.as_str().and_then(normalize_text) {
        return vec![google_text_part(text)];
    }

    if let Some(items) = content.as_array() {
        let mut parts = Vec::new();
        for item in items {
            if let Some(part) = google_content_part(item, messages) {
                parts.push(part);
            }
        }
        return parts;
    }

    if content.is_null() {
        return Vec::new();
    }

    normalize_text(content.to_string().as_str())
        .map(|text| vec![google_text_part(text)])
        .unwrap_or_default()
}

fn google_content_part(value: &Value, messages: &[Value]) -> Option<Value> {
    if let Some(text) = value.as_str().and_then(normalize_text) {
        return Some(google_text_part(text));
    }

    if let Some(function_call) = value.get("functionCall").cloned() {
        return Some(json!({ "functionCall": function_call }));
    }
    if let Some(function_response) = value.get("functionResponse").cloned() {
        return Some(json!({ "functionResponse": function_response }));
    }

    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        if kind == "text"
            && let Some(text) = value.get("text").and_then(content_text_value)
        {
            return Some(google_text_part(text));
        }
        if kind == "tool_use" {
            return google_function_call_part_from_native(value);
        }
        if kind == "tool_result" {
            return google_function_response_part_from_native(messages, value);
        }
    }

    if let Some(text) = value.get("text").and_then(content_text_value) {
        return Some(google_text_part(text));
    }

    None
}

fn google_text_part(text: String) -> Value {
    json!({
        "text": text
    })
}

fn google_function_call_part_from_openai(tool_call: &Value) -> Option<Value> {
    let function = tool_call.get("function")?;
    let name = function.get("name")?.as_str()?;
    let arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let args = serde_json::from_str::<Value>(arguments)
        .ok()
        .unwrap_or_else(|| json!({}));
    Some(json!({
        "functionCall": {
            "name": name,
            "args": args,
        }
    }))
}

fn google_function_call_part_from_native(value: &Value) -> Option<Value> {
    let name = value.get("name").and_then(Value::as_str)?;
    let args = value.get("input").cloned().unwrap_or_else(|| json!({}));
    Some(json!({
        "functionCall": {
            "name": name,
            "args": args,
        }
    }))
}

fn google_function_response_part_from_native(messages: &[Value], value: &Value) -> Option<Value> {
    let name = resolve_google_tool_name_for_native_result(messages, value)?;
    let response_source = value
        .get("result")
        .or_else(|| value.get("content"))
        .unwrap_or(&Value::Null);
    let response = google_tool_response_payload(response_source);

    Some(json!({
        "functionResponse": {
            "name": name,
            "response": response,
        }
    }))
}

fn google_tool_response_payload(content: &Value) -> Value {
    if content.is_object() || content.is_array() {
        return content.clone();
    }

    let Some(text) = content_as_text(content) else {
        return json!({
            "result": ""
        });
    };
    serde_json::from_str::<Value>(text.as_str())
        .ok()
        .unwrap_or_else(|| json!({ "result": text }))
}

fn resolve_google_tool_name_for_result<'a>(
    messages: &'a [Value],
    tool_call_id: &str,
) -> Option<&'a str> {
    if tool_call_id.is_empty() {
        return None;
    }

    for message in messages.iter().rev() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role != "assistant" {
            continue;
        }
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                let current_id = tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if current_id != tool_call_id {
                    continue;
                }
                let function = tool_call.get("function")?;
                let name = function.get("name").and_then(Value::as_str)?;
                return Some(name);
            }
        }
    }

    None
}

fn resolve_google_tool_name_for_native_result(messages: &[Value], value: &Value) -> Option<String> {
    if let Some(name) = value.get("name").and_then(Value::as_str) {
        return Some(name.to_owned());
    }

    let tool_use_id = value
        .get("tool_use_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)?;

    for message in messages.iter().rev() {
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };

        for item in content.iter().rev() {
            let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
            if kind != "tool_use" {
                continue;
            }

            let current_id = item
                .get("id")
                .or_else(|| item.get("tool_use_id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if current_id != tool_use_id {
                continue;
            }

            let name = item.get("name").and_then(Value::as_str)?;
            return Some(name.to_owned());
        }
    }

    None
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

fn google_tool_declarations(tool_definitions: &[Value]) -> Vec<Value> {
    tool_definitions
        .iter()
        .filter_map(openai_tool_definition_to_google)
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

fn openai_tool_definition_to_google(tool_definition: &Value) -> Option<Value> {
    let function = tool_definition.get("function")?;
    let name = function.get("name")?.as_str()?;
    let description = function
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parameters = function.get("parameters").cloned().unwrap_or_else(|| {
        json!({
            "type": "object",
            "properties": {},
        })
    });
    Some(json!({
        "name": name,
        "description": description,
        "parameters": parameters,
    }))
}

fn build_responses_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
    streaming: bool,
) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("stream".to_owned(), Value::Bool(streaming));

    let (instructions, input_items) = build_responses_input_items(messages);
    if let Some(instructions) = instructions {
        body.insert("instructions".to_owned(), json!(instructions));
    }
    body.insert("input".to_owned(), Value::Array(input_items));
    apply_common_reasoning_and_temperature_fields(&mut body, config, payload_mode);

    if let Some(limit) = config.provider.max_tokens {
        match payload_mode.token_field {
            TokenLimitField::MaxOutputTokens => {
                body.insert("max_output_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::MaxCompletionTokens => {
                body.insert("max_completion_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::MaxTokens => {
                body.insert("max_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::Omit => {}
        }
    }

    if include_tool_schema && !tool_definitions.is_empty() {
        body.insert("tools".to_owned(), Value::Array(tool_definitions.to_vec()));
        body.insert("tool_choice".to_owned(), json!("auto"));
    }

    Value::Object(body)
}

fn build_responses_input_items(messages: &[Value]) -> (Option<String>, Vec<Value>) {
    let mut instructions = Vec::new();
    let mut input_items = Vec::new();
    let mut seen_non_system_message = false;

    for message in messages {
        if let Some(native_item) = normalize_responses_native_input_item(message) {
            seen_non_system_message = true;
            input_items.push(native_item);
            continue;
        }

        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        let Some(text) = extract_request_message_text(message.get("content")) else {
            continue;
        };
        if role == "system" && !seen_non_system_message {
            instructions.push(text);
            continue;
        }
        seen_non_system_message = true;
        input_items.push(json!({
            "role": role,
            "content": [{
                "type": "input_text",
                "text": text,
            }],
        }));
    }

    let merged_instructions = if instructions.is_empty() {
        None
    } else {
        Some(instructions.join("\n\n"))
    };

    (merged_instructions, input_items)
}

fn normalize_responses_native_input_item(message: &Value) -> Option<Value> {
    let item_type = message.get("type").and_then(Value::as_str)?;
    match item_type {
        "function_call" | "function_call_output" | "reasoning" => Some(message.clone()),
        _ => None,
    }
}

fn extract_request_message_text(content: Option<&Value>) -> Option<String> {
    let content = content?;
    if let Some(text) = content.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_owned());
    }

    let parts = content.as_array()?;
    let mut merged = Vec::new();
    for part in parts {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                merged.push(trimmed.to_owned());
            }
            continue;
        }
        if let Some(text) = part
            .get("text")
            .and_then(|value| value.get("value"))
            .and_then(Value::as_str)
        {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                merged.push(trimmed.to_owned());
            }
        }
    }

    if merged.is_empty() {
        return None;
    }
    Some(merged.join("\n"))
}

fn apply_common_payload_fields(
    body: &mut serde_json::Map<String, Value>,
    config: &LoongClawConfig,
    payload_mode: CompletionPayloadMode,
    capability: ProviderCapabilityContract,
) {
    apply_common_reasoning_and_temperature_fields(body, config, payload_mode);

    if let Some(limit) = config.provider.max_tokens {
        match payload_mode.token_field {
            TokenLimitField::MaxCompletionTokens => {
                body.insert("max_completion_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::MaxTokens => {
                body.insert("max_tokens".to_owned(), json!(limit));
            }
            TokenLimitField::MaxOutputTokens | TokenLimitField::Omit => {}
        }
    }

    if capability.include_reasoning_extra_body()
        && let Some(extra_body) = kimi_extra_body(config.provider.reasoning_effort)
    {
        body.insert("extra_body".to_owned(), extra_body);
    }
}

fn apply_common_reasoning_and_temperature_fields(
    body: &mut serde_json::Map<String, Value>,
    config: &LoongClawConfig,
    payload_mode: CompletionPayloadMode,
) {
    if payload_mode.temperature_field == TemperatureField::Include {
        body.insert("temperature".to_owned(), json!(config.provider.temperature));
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
