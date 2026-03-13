use serde_json::{Value, json};

use crate::config::{LoongClawConfig, ReasoningEffort};

use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{
    CompletionPayloadMode, ProviderCapabilityContract, ReasoningField, TemperatureField,
    TokenLimitField, provider_runtime_contract,
};

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
    let mut body = build_completion_request_body_with_capability(
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
