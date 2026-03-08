use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::sleep;

use crate::CliResult;

use super::config::{LoongClawConfig, ProviderConfig, ProviderKind};
#[cfg(feature = "memory-sqlite")]
use super::memory;

mod policy;
mod shape;
mod transport;

pub fn build_messages_for_session(
    config: &LoongClawConfig,
    session_id: &str,
    include_system_prompt: bool,
) -> CliResult<Vec<Value>> {
    let mut messages = Vec::new();
    if include_system_prompt {
        let system = config.cli.system_prompt.trim();
        if !system.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": system,
            }));
        }
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let turns = memory::window_direct(session_id, config.memory.sliding_window)
            .map_err(|error| format!("load memory window failed: {error}"))?;
        for turn in turns {
            messages.push(json!({
                "role": turn.role,
                "content": turn.content,
            }));
        }
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = session_id;
    }
    Ok(messages)
}

pub async fn request_completion(config: &LoongClawConfig, messages: &[Value]) -> CliResult<String> {
    validate_provider_feature_gate(config)?;

    let endpoint = config.provider.endpoint();
    let headers = transport::build_request_headers(&config.provider.headers)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let client = build_http_client(&request_policy)?;
    let model_candidates = resolve_request_models(config, &headers, &request_policy).await?;
    let auto_model_mode = config.provider.model_selection_requires_fetch();

    let mut last_error = None;
    for (index, model) in model_candidates.iter().enumerate() {
        match request_completion_with_model(
            config,
            messages,
            model,
            auto_model_mode,
            &endpoint,
            &headers,
            &request_policy,
            &client,
        )
        .await
        {
            Ok(content) => return Ok(content),
            Err(model_error) => {
                if model_error.try_next_model && index + 1 < model_candidates.len() {
                    last_error = Some(model_error.message);
                    continue;
                }
                return Err(model_error.message);
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| "provider request failed for every model candidate".to_owned()))
}

pub async fn fetch_available_models(config: &LoongClawConfig) -> CliResult<Vec<String>> {
    validate_provider_feature_gate(config)?;
    let headers = transport::build_request_headers(&config.provider.headers)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    fetch_available_models_with_policy(config, &headers, &request_policy).await
}

fn build_http_client(request_policy: &policy::ProviderRequestPolicy) -> CliResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(request_policy.timeout_ms))
        .build()
        .map_err(|error| format!("build provider http client failed: {error}"))
}

fn build_completion_request_body(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    payload_mode: CompletionPayloadMode,
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

    Value::Object(body)
}

async fn resolve_request_models(
    config: &LoongClawConfig,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
) -> CliResult<Vec<String>> {
    if !config.provider.model_selection_requires_fetch() {
        return Ok(vec![config.provider.model.trim().to_owned()]);
    }
    let available = fetch_available_models_with_policy(config, headers, request_policy).await?;
    let ordered = rank_model_candidates(&config.provider, &available);
    if ordered.is_empty() {
        return Err("provider model-list is empty; set provider.model explicitly".to_owned());
    }
    Ok(ordered)
}

async fn fetch_available_models_with_policy(
    config: &LoongClawConfig,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
) -> CliResult<Vec<String>> {
    let endpoint = config.provider.models_endpoint();
    let client = build_http_client(request_policy)?;

    let mut attempt = 0usize;
    let mut backoff_ms = request_policy.initial_backoff_ms;
    loop {
        attempt += 1;
        let mut req = client.get(endpoint.clone()).headers(headers.clone());
        if let Some(auth_header) = config.provider.authorization_header() {
            req = req.header(reqwest::header::AUTHORIZATION, auth_header);
        }

        match req.send().await {
            Ok(response) => {
                let status = response.status();
                let response_body = transport::decode_response_body(response)
                    .await
                    .map_err(|error| {
                        format!(
                            "provider model-list decode failed on attempt {attempt}/{max_attempts}: {error}",
                            max_attempts = request_policy.max_attempts
                        )
                    })?;

                if status.is_success() {
                    let models = shape::extract_model_ids(&response_body);
                    if models.is_empty() {
                        return Err(format!(
                            "provider model-list returned no models from endpoint `{endpoint}`"
                        ));
                    }
                    return Ok(models);
                }

                let status_code = status.as_u16();
                if attempt < request_policy.max_attempts && policy::should_retry_status(status_code)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(backoff_ms, request_policy.max_backoff_ms);
                    continue;
                }

                return Err(format!(
                    "provider model-list returned status {status_code} on attempt {attempt}/{max_attempts}: {response_body}",
                    max_attempts = request_policy.max_attempts
                ));
            }
            Err(error) => {
                if attempt < request_policy.max_attempts && policy::should_retry_error(&error) {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(backoff_ms, request_policy.max_backoff_ms);
                    continue;
                }
                return Err(format!(
                    "provider model-list request failed on attempt {attempt}/{max_attempts}: {error}",
                    max_attempts = request_policy.max_attempts
                ));
            }
        }
    }
}

fn rank_model_candidates(provider: &ProviderConfig, available: &[String]) -> Vec<String> {
    let mut ordered = Vec::new();
    for raw in &provider.preferred_models {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(matched) = available.iter().find(|model| *model == trimmed) {
            push_unique_model(&mut ordered, matched);
            continue;
        }
        if let Some(matched) = available
            .iter()
            .find(|model| model.eq_ignore_ascii_case(trimmed))
        {
            push_unique_model(&mut ordered, matched);
        }
    }

    for model in available {
        push_unique_model(&mut ordered, model);
    }
    ordered
}

fn push_unique_model(out: &mut Vec<String>, model: &str) {
    if out.iter().any(|existing| existing == model) {
        return;
    }
    out.push(model.to_owned());
}

#[derive(Debug)]
struct ModelRequestError {
    message: String,
    try_next_model: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenLimitField {
    MaxCompletionTokens,
    MaxTokens,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReasoningField {
    ReasoningEffort,
    ReasoningObject,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemperatureField {
    Include,
    Omit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompletionPayloadMode {
    token_field: TokenLimitField,
    reasoning_field: ReasoningField,
    temperature_field: TemperatureField,
}

impl CompletionPayloadMode {
    fn default_for(provider: &ProviderConfig) -> Self {
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
struct ProviderApiError {
    code: Option<String>,
    param: Option<String>,
    message: Option<String>,
}

fn parse_provider_api_error(body: &Value) -> ProviderApiError {
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

fn should_try_next_model_on_error(error: &ProviderApiError) -> bool {
    if let Some(code) = error.code.as_deref() {
        if matches!(
            code,
            "model_not_found" | "unsupported_model" | "invalid_model" | "not_found_error"
        ) {
            return true;
        }
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

fn adapt_payload_mode_for_error(
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

#[allow(clippy::too_many_arguments)]
async fn request_completion_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
) -> Result<String, ModelRequestError> {
    let mut attempt = 0usize;
    let mut backoff_ms = request_policy.initial_backoff_ms;
    let mut payload_mode = CompletionPayloadMode::default_for(&config.provider);
    let mut tried_payload_modes = vec![payload_mode];

    loop {
        attempt += 1;
        let body = build_completion_request_body(config, messages, model, payload_mode);
        let mut req = client.post(endpoint).headers(headers.clone()).json(&body);
        if let Some(auth_header) = config.provider.authorization_header() {
            req = req.header(reqwest::header::AUTHORIZATION, auth_header);
        }

        match req.send().await {
            Ok(response) => {
                let status = response.status();
                let response_body = transport::decode_response_body(response)
                    .await
                    .map_err(|error| ModelRequestError {
                        message: format!(
                            "provider response decode failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                            max_attempts = request_policy.max_attempts
                        ),
                        try_next_model: false,
                    })?;

                if status.is_success() {
                    let content = shape::extract_message_content(&response_body).ok_or_else(|| {
                        ModelRequestError {
                            message: format!(
                                "provider response missing choices[0].message.content for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                                max_attempts = request_policy.max_attempts
                            ),
                            try_next_model: false,
                        }
                    })?;
                    return Ok(content);
                }

                let api_error = parse_provider_api_error(&response_body);
                if let Some(next_mode) =
                    adapt_payload_mode_for_error(payload_mode, &config.provider, &api_error)
                {
                    if !tried_payload_modes.contains(&next_mode) {
                        payload_mode = next_mode;
                        tried_payload_modes.push(next_mode);
                        continue;
                    }
                }

                let status_code = status.as_u16();
                if attempt < request_policy.max_attempts && policy::should_retry_status(status_code)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(backoff_ms, request_policy.max_backoff_ms);
                    continue;
                }

                if auto_model_mode && should_try_next_model_on_error(&api_error) {
                    return Err(ModelRequestError {
                        message: format!(
                            "model `{model}` rejected by provider endpoint; trying next candidate. status {status_code}: {response_body}"
                        ),
                        try_next_model: true,
                    });
                }

                return Err(ModelRequestError {
                    message: format!(
                        "provider returned status {status_code} for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                        max_attempts = request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
            Err(error) => {
                if attempt < request_policy.max_attempts && policy::should_retry_error(&error) {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(backoff_ms, request_policy.max_backoff_ms);
                    continue;
                }
                return Err(ModelRequestError {
                    message: format!(
                        "provider request failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                        max_attempts = request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
        }
    }
}

fn validate_provider_feature_gate(config: &LoongClawConfig) -> CliResult<()> {
    match config.provider.kind {
        ProviderKind::Volcengine => {
            if !cfg!(feature = "provider-volcengine") {
                return Err(
                    "volcengine provider is disabled (enable feature `provider-volcengine`)"
                        .to_owned(),
                );
            }
        }
        _ => {
            if !cfg!(feature = "provider-openai") {
                return Err(
                    "openai-compatible provider family is disabled (enable feature `provider-openai`)"
                        .to_owned(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        FeishuChannelConfig, MemoryConfig, ProviderConfig, ReasoningEffort, ToolConfig,
    };
    use serde_json::json;

    #[test]
    fn message_builder_includes_system_prompt() {
        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        assert!(!messages.is_empty());
        assert_eq!(messages[0]["role"], "system");
    }

    #[test]
    fn completion_body_includes_reasoning_effort_when_configured() {
        let mut config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };
        config.provider.reasoning_effort = Some(ReasoningEffort::High);

        let body = build_completion_request_body(
            &config,
            &[],
            "model-latest",
            CompletionPayloadMode::default_for(&config.provider),
        );
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn model_catalog_selection_prefers_user_preferences() {
        let config = ProviderConfig {
            model: "auto".to_owned(),
            preferred_models: vec!["model-latest".to_owned(), "model-fallback".to_owned()],
            ..ProviderConfig::default()
        };
        let ranked = rank_model_candidates(
            &config,
            &["model-fallback".to_owned(), "model-latest".to_owned()],
        );
        let selected = ranked.first().expect("model selected");
        assert_eq!(selected, "model-latest");
    }

    #[test]
    fn completion_body_omits_optional_fields_when_not_configured() {
        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let body = build_completion_request_body(
            &config,
            &[],
            "model-latest",
            CompletionPayloadMode::default_for(&config.provider),
        );
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("reasoning").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn completion_body_uses_provider_token_field_default() {
        let openai = ProviderConfig {
            kind: ProviderKind::Openai,
            max_tokens: Some(512),
            ..ProviderConfig::default()
        };
        let openai_mode = CompletionPayloadMode::default_for(&openai);
        assert_eq!(
            openai_mode.token_field,
            TokenLimitField::MaxCompletionTokens
        );

        let openrouter = ProviderConfig {
            kind: ProviderKind::Openrouter,
            max_tokens: Some(512),
            ..ProviderConfig::default()
        };
        let openrouter_mode = CompletionPayloadMode::default_for(&openrouter);
        assert_eq!(openrouter_mode.token_field, TokenLimitField::MaxTokens);
    }

    #[test]
    fn payload_mode_adapts_for_parameter_incompatibility() {
        let provider = ProviderConfig {
            max_tokens: Some(1024),
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..ProviderConfig::default()
        };

        let max_tokens_error = json!({
            "error": {
                "code": "unsupported_parameter",
                "param": "max_tokens",
                "message": "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead."
            }
        });
        let reasoning_effort_error = json!({
            "error": {
                "code": "unknown_parameter",
                "param": "reasoning_effort",
                "message": "Unknown parameter: 'reasoning_effort'."
            }
        });

        let mut mode = CompletionPayloadMode {
            token_field: TokenLimitField::MaxTokens,
            reasoning_field: ReasoningField::ReasoningEffort,
            temperature_field: TemperatureField::Include,
        };

        mode = adapt_payload_mode_for_error(
            mode,
            &provider,
            &parse_provider_api_error(&max_tokens_error),
        )
        .expect("max_tokens adapt");
        assert_eq!(mode.token_field, TokenLimitField::MaxCompletionTokens);

        mode = adapt_payload_mode_for_error(
            mode,
            &provider,
            &parse_provider_api_error(&reasoning_effort_error),
        )
        .expect("reasoning adapt");
        assert_eq!(mode.reasoning_field, ReasoningField::ReasoningObject);
    }

    #[test]
    fn payload_mode_can_drop_temperature_when_model_rejects_it() {
        let provider = ProviderConfig::default();
        let unsupported_temperature = json!({
            "error": {
                "code": "unsupported_value",
                "param": "temperature",
                "message": "Only the default (1) value is supported."
            }
        });

        let mode = CompletionPayloadMode::default_for(&provider);
        let adapted = adapt_payload_mode_for_error(
            mode,
            &provider,
            &parse_provider_api_error(&unsupported_temperature),
        )
        .expect("temperature adaptation");
        assert_eq!(adapted.temperature_field, TemperatureField::Omit);
    }

    #[test]
    fn ranking_model_candidates_keeps_preferences_then_catalog() {
        let config = ProviderConfig {
            model: "auto".to_owned(),
            preferred_models: vec![
                "model-z".to_owned(),
                "MODEL-A".to_owned(),
                "model-z".to_owned(),
            ],
            ..ProviderConfig::default()
        };

        let ranked = rank_model_candidates(
            &config,
            &[
                "model-a".to_owned(),
                "model-b".to_owned(),
                "model-z".to_owned(),
            ],
        );
        assert_eq!(ranked, vec!["model-z", "model-a", "model-b"]);
    }

    #[test]
    fn model_error_parser_detects_endpoint_mismatch() {
        let body = json!({
            "error": {
                "message": "The model `gpt-5.4-pro` only supports /v1/responses and not this endpoint."
            }
        });
        let parsed = parse_provider_api_error(&body);
        assert!(should_try_next_model_on_error(&parsed));
    }
}
