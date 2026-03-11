use serde_json::{Value, json};

use crate::config::{LoongClawConfig, ProviderConfig, ProviderKind, ReasoningEffort};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderTransportMode {
    OpenAiChatCompletions,
    KimiApi,
}

impl ProviderTransportMode {
    fn for_provider(provider: &ProviderConfig) -> Self {
        match provider.kind {
            ProviderKind::KimiCoding => Self::KimiApi,
            _ => Self::OpenAiChatCompletions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenLimitField {
    MaxCompletionTokens,
    MaxTokens,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReasoningField {
    ReasoningEffort,
    ReasoningObject,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemperatureField {
    Include,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CompletionPayloadMode {
    pub(super) token_field: TokenLimitField,
    pub(super) reasoning_field: ReasoningField,
    pub(super) temperature_field: TemperatureField,
}

impl CompletionPayloadMode {
    pub(super) fn default_for(provider: &ProviderConfig) -> Self {
        let token_field = if provider.max_tokens.is_some() {
            if provider.kind == ProviderKind::Openai {
                TokenLimitField::MaxCompletionTokens
            } else {
                TokenLimitField::MaxTokens
            }
        } else {
            TokenLimitField::Omit
        };

        let reasoning_field = if provider.reasoning_effort.is_some() {
            ReasoningField::ReasoningEffort
        } else {
            ReasoningField::Omit
        };

        Self {
            token_field,
            reasoning_field,
            temperature_field: TemperatureField::Include,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ProviderApiError {
    pub(super) code: Option<String>,
    pub(super) param: Option<String>,
    pub(super) message: Option<String>,
}

pub(super) fn build_completion_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
) -> Value {
    let transport_mode = ProviderTransportMode::for_provider(&config.provider);
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

    if transport_mode == ProviderTransportMode::KimiApi
        && let Some(extra_body) = kimi_extra_body(config.provider.reasoning_effort)
    {
        body.insert("extra_body".to_owned(), extra_body);
    }

    Value::Object(body)
}

pub(super) fn build_turn_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
    include_tool_schema: bool,
    tool_definitions: &[Value],
) -> Value {
    let mut body = build_completion_request_body(config, messages, model, payload_mode);
    if include_tool_schema
        && !tool_definitions.is_empty()
        && let Some(object) = body.as_object_mut()
    {
        object.insert("tools".to_owned(), Value::Array(tool_definitions.to_vec()));
        object.insert("tool_choice".to_owned(), json!("auto"));
    }
    body
}

pub(super) fn parse_provider_api_error(body: &Value) -> ProviderApiError {
    let error = body.get("error").unwrap_or(body);
    ProviderApiError {
        code: error
            .get("code")
            .and_then(Value::as_str)
            .map(str::to_lowercase),
        param: error
            .get("param")
            .and_then(Value::as_str)
            .map(str::to_lowercase),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_lowercase),
    }
}

fn is_parameter_unsupported(error: &ProviderApiError, parameter: &str) -> bool {
    let param = parameter.to_lowercase();
    if error.param.as_deref() == Some(param.as_str()) {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    if !(message.contains("unknown parameter")
        || message.contains("unsupported parameter")
        || message.contains("not supported"))
    {
        return false;
    }
    message.contains(param.as_str())
}

pub(super) fn should_disable_tool_schema_for_error(error: &ProviderApiError) -> bool {
    if is_parameter_unsupported(error, "tools") || is_parameter_unsupported(error, "tool_choice") {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    message.contains("tools are not supported")
        || message.contains("tool use is not supported")
        || message.contains("function calling is not supported")
}

pub(super) fn should_try_next_model_on_error(error: &ProviderApiError) -> bool {
    if let Some(code) = error.code.as_deref()
        && matches!(
            code,
            "model_not_found" | "unsupported_model" | "invalid_model" | "not_found_error"
        )
    {
        return true;
    }
    if error.param.as_deref() == Some("model") {
        return true;
    }

    let message = error.message.as_deref().unwrap_or_default();
    message.contains("only supports")
        || message.contains("only available")
        || message.contains("this endpoint")
        || message.contains("/v1/responses")
        || message.contains("model does not exist")
}

pub(super) fn adapt_payload_mode_for_error(
    current: CompletionPayloadMode,
    provider: &ProviderConfig,
    error: &ProviderApiError,
) -> Option<CompletionPayloadMode> {
    if provider.max_tokens.is_some() {
        if is_parameter_unsupported(error, "max_tokens") {
            return Some(match current.token_field {
                TokenLimitField::MaxTokens => CompletionPayloadMode {
                    token_field: TokenLimitField::MaxCompletionTokens,
                    ..current
                },
                TokenLimitField::MaxCompletionTokens => CompletionPayloadMode {
                    token_field: TokenLimitField::Omit,
                    ..current
                },
                TokenLimitField::Omit => current,
            });
        }

        if is_parameter_unsupported(error, "max_completion_tokens") {
            return Some(match current.token_field {
                TokenLimitField::MaxCompletionTokens => CompletionPayloadMode {
                    token_field: TokenLimitField::MaxTokens,
                    ..current
                },
                TokenLimitField::MaxTokens => CompletionPayloadMode {
                    token_field: TokenLimitField::Omit,
                    ..current
                },
                TokenLimitField::Omit => current,
            });
        }
    }

    if provider.reasoning_effort.is_some() {
        if is_parameter_unsupported(error, "reasoning_effort") {
            return Some(match current.reasoning_field {
                ReasoningField::ReasoningEffort => CompletionPayloadMode {
                    reasoning_field: ReasoningField::ReasoningObject,
                    ..current
                },
                ReasoningField::ReasoningObject => CompletionPayloadMode {
                    reasoning_field: ReasoningField::Omit,
                    ..current
                },
                ReasoningField::Omit => current,
            });
        }

        if is_parameter_unsupported(error, "reasoning") {
            return Some(match current.reasoning_field {
                ReasoningField::ReasoningObject => CompletionPayloadMode {
                    reasoning_field: ReasoningField::ReasoningEffort,
                    ..current
                },
                ReasoningField::ReasoningEffort => CompletionPayloadMode {
                    reasoning_field: ReasoningField::Omit,
                    ..current
                },
                ReasoningField::Omit => current,
            });
        }
    }

    let temperature_rejected = is_parameter_unsupported(error, "temperature")
        || (error.param.as_deref() == Some("temperature")
            && error
                .message
                .as_deref()
                .unwrap_or_default()
                .contains("only the default"));
    if temperature_rejected {
        return Some(match current.temperature_field {
            TemperatureField::Include => CompletionPayloadMode {
                temperature_field: TemperatureField::Omit,
                ..current
            },
            TemperatureField::Omit => current,
        });
    }

    None
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
