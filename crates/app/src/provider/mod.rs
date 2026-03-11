use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::sleep;

use crate::CliResult;

use super::config::LoongClawConfig;
#[cfg(feature = "memory-sqlite")]
use super::memory;

mod error_policy;
mod model_selection;
mod payload_adaptation;
mod policy;
mod shape;
mod transport;

use error_policy::{
    provider_request_failed_for_all_models, validate_provider_configuration,
    validate_provider_feature_gate,
};
use model_selection::{fetch_available_models_with_policy, resolve_request_models};
use payload_adaptation::{
    CompletionPayloadMode, adapt_payload_mode_for_error, build_completion_request_body,
    build_turn_request_body, parse_provider_api_error, should_disable_tool_schema_for_error,
    should_try_next_model_on_error,
};

pub use shape::extract_provider_turn;

pub fn build_system_message(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Option<Value> {
    if !include_system_prompt {
        return None;
    }
    let system = config.cli.system_prompt.trim();
    let snapshot = super::tools::capability_snapshot();
    let content = if system.is_empty() {
        snapshot
    } else {
        format!("{system}\n\n{snapshot}")
    };
    Some(json!({
        "role": "system",
        "content": content,
    }))
}

pub(crate) fn build_base_messages(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Vec<Value> {
    build_system_message(config, include_system_prompt)
        .into_iter()
        .collect()
}

pub(crate) fn push_history_message(messages: &mut Vec<Value>, role: &str, content: &str) {
    if !is_supported_chat_role(role) {
        return;
    }
    if should_skip_history_turn(role, content) {
        return;
    }
    messages.push(json!({
        "role": role,
        "content": content,
    }));
}

fn is_supported_chat_role(role: &str) -> bool {
    matches!(role, "system" | "user" | "assistant" | "tool")
}

fn should_skip_history_turn(role: &str, content: &str) -> bool {
    if role != "assistant" {
        return false;
    }
    let parsed = match serde_json::from_str::<Value>(content) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let event_type = parsed
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(
        event_type,
        "conversation_event" | "tool_decision" | "tool_outcome"
    )
}

pub fn load_memory_window_messages(
    config: &LoongClawConfig,
    session_id: &str,
) -> CliResult<Vec<Value>> {
    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config = super::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
            sliding_window: Some(config.memory.sliding_window),
        };
        let turns = memory::window_direct(session_id, config.memory.sliding_window, &mem_config)
            .map_err(|error| format!("load memory window failed: {error}"))?;
        let mut messages = Vec::with_capacity(turns.len());
        for turn in turns {
            push_history_message(&mut messages, turn.role.as_str(), turn.content.as_str());
        }
        Ok(messages)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id);
        Ok(Vec::new())
    }
}

pub fn build_messages_for_session(
    config: &LoongClawConfig,
    session_id: &str,
    include_system_prompt: bool,
) -> CliResult<Vec<Value>> {
    let mut messages = build_base_messages(config, include_system_prompt);
    messages.extend(load_memory_window_messages(config, session_id)?);
    Ok(messages)
}

pub async fn request_completion(config: &LoongClawConfig, messages: &[Value]) -> CliResult<String> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;

    let endpoint = config.provider.endpoint();
    let headers = transport::build_request_headers(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let client = build_http_client(&request_policy)?;
    let model_candidates = resolve_request_models(config, &headers, &request_policy).await?;
    let request_context = ProviderRequestContext {
        endpoint: &endpoint,
        headers: &headers,
        request_policy: &request_policy,
        client: &client,
        auto_model_mode: config.provider.model_selection_requires_fetch(),
    };

    let mut last_error = None;
    for (index, model) in model_candidates.iter().enumerate() {
        match request_completion_with_model(config, messages, model, &request_context).await {
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
    Err(provider_request_failed_for_all_models(last_error))
}

pub async fn request_turn(
    config: &LoongClawConfig,
    messages: &[Value],
) -> CliResult<crate::conversation::turn_engine::ProviderTurn> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;

    let endpoint = config.provider.endpoint();
    let headers = transport::build_request_headers(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let client = build_http_client(&request_policy)?;
    let model_candidates = resolve_request_models(config, &headers, &request_policy).await?;
    let request_context = ProviderRequestContext {
        endpoint: &endpoint,
        headers: &headers,
        request_policy: &request_policy,
        client: &client,
        auto_model_mode: config.provider.model_selection_requires_fetch(),
    };

    let mut last_error = None;
    for (index, model) in model_candidates.iter().enumerate() {
        match request_turn_with_model(config, messages, model, &request_context).await {
            Ok(turn) => return Ok(turn),
            Err(model_error) => {
                if model_error.try_next_model && index + 1 < model_candidates.len() {
                    last_error = Some(model_error.message);
                    continue;
                }
                return Err(model_error.message);
            }
        }
    }
    Err(provider_request_failed_for_all_models(last_error))
}

pub async fn fetch_available_models(config: &LoongClawConfig) -> CliResult<Vec<String>> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;
    let headers = transport::build_request_headers(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    fetch_available_models_with_policy(config, &headers, &request_policy).await
}

fn build_http_client(request_policy: &policy::ProviderRequestPolicy) -> CliResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(request_policy.timeout_ms))
        .build()
        .map_err(|error| format!("build provider http client failed: {error}"))
}

#[derive(Debug)]
struct ModelRequestError {
    message: String,
    try_next_model: bool,
}

struct ProviderRequestContext<'a> {
    endpoint: &'a str,
    headers: &'a reqwest::header::HeaderMap,
    request_policy: &'a policy::ProviderRequestPolicy,
    client: &'a reqwest::Client,
    auto_model_mode: bool,
}

async fn request_completion_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    request_context: &ProviderRequestContext<'_>,
) -> Result<String, ModelRequestError> {
    let mut attempt = 0usize;
    let mut backoff_ms = request_context.request_policy.initial_backoff_ms;
    let mut payload_mode = CompletionPayloadMode::default_for(&config.provider);
    let mut tried_payload_modes = vec![payload_mode];

    loop {
        attempt += 1;
        let body = build_completion_request_body(config, messages, model, payload_mode);
        let mut req = request_context
            .client
            .post(request_context.endpoint)
            .headers(request_context.headers.clone())
            .json(&body);
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
                            max_attempts = request_context.request_policy.max_attempts
                        ),
                        try_next_model: false,
                    })?;

                if status.is_success() {
                    let content = shape::extract_message_content(&response_body).ok_or_else(|| {
                        ModelRequestError {
                            message: format!(
                                "provider response missing choices[0].message.content for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                                max_attempts = request_context.request_policy.max_attempts
                            ),
                            try_next_model: false,
                        }
                    })?;
                    return Ok(content);
                }

                let api_error = parse_provider_api_error(&response_body);
                if let Some(next_mode) =
                    adapt_payload_mode_for_error(payload_mode, &config.provider, &api_error)
                    && !tried_payload_modes.contains(&next_mode)
                {
                    payload_mode = next_mode;
                    tried_payload_modes.push(next_mode);
                    continue;
                }

                let status_code = status.as_u16();
                if attempt < request_context.request_policy.max_attempts
                    && policy::should_retry_status(status_code)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(
                        backoff_ms,
                        request_context.request_policy.max_backoff_ms,
                    );
                    continue;
                }

                if request_context.auto_model_mode && should_try_next_model_on_error(&api_error) {
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
                        max_attempts = request_context.request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
            Err(error) => {
                if attempt < request_context.request_policy.max_attempts
                    && policy::should_retry_error(&error)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(
                        backoff_ms,
                        request_context.request_policy.max_backoff_ms,
                    );
                    continue;
                }
                return Err(ModelRequestError {
                    message: format!(
                        "provider request failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                        max_attempts = request_context.request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
        }
    }
}

async fn request_turn_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    request_context: &ProviderRequestContext<'_>,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let mut attempt = 0usize;
    let mut backoff_ms = request_context.request_policy.initial_backoff_ms;
    let mut payload_mode = CompletionPayloadMode::default_for(&config.provider);
    let mut tried_payload_modes = vec![payload_mode];
    let tool_definitions = super::tools::provider_tool_definitions();
    let mut include_tool_schema = !tool_definitions.is_empty();

    loop {
        attempt += 1;
        let body = build_turn_request_body(
            config,
            messages,
            model,
            payload_mode,
            include_tool_schema,
            &tool_definitions,
        );
        let mut req = request_context
            .client
            .post(request_context.endpoint)
            .headers(request_context.headers.clone())
            .json(&body);
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
                            max_attempts = request_context.request_policy.max_attempts
                        ),
                        try_next_model: false,
                    })?;

                if status.is_success() {
                    let turn = shape::extract_provider_turn(&response_body).ok_or_else(|| {
                        ModelRequestError {
                            message: format!(
                                "provider response missing choices[0].message for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                                max_attempts = request_context.request_policy.max_attempts
                            ),
                            try_next_model: false,
                        }
                    })?;
                    return Ok(turn);
                }

                let api_error = parse_provider_api_error(&response_body);
                if include_tool_schema && should_disable_tool_schema_for_error(&api_error) {
                    include_tool_schema = false;
                    continue;
                }
                if let Some(next_mode) =
                    adapt_payload_mode_for_error(payload_mode, &config.provider, &api_error)
                    && !tried_payload_modes.contains(&next_mode)
                {
                    payload_mode = next_mode;
                    tried_payload_modes.push(next_mode);
                    continue;
                }

                let status_code = status.as_u16();
                if attempt < request_context.request_policy.max_attempts
                    && policy::should_retry_status(status_code)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(
                        backoff_ms,
                        request_context.request_policy.max_backoff_ms,
                    );
                    continue;
                }

                if request_context.auto_model_mode && should_try_next_model_on_error(&api_error) {
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
                        max_attempts = request_context.request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
            Err(error) => {
                if attempt < request_context.request_policy.max_attempts
                    && policy::should_retry_error(&error)
                {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = policy::next_backoff_ms(
                        backoff_ms,
                        request_context.request_policy.max_backoff_ms,
                    );
                    continue;
                }
                return Err(ModelRequestError {
                    message: format!(
                        "provider request failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                        max_attempts = request_context.request_policy.max_attempts
                    ),
                    try_next_model: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::model_selection::rank_model_candidates;
    use super::payload_adaptation::{ReasoningField, TemperatureField, TokenLimitField};
    use super::*;
    use crate::config::{
        ConversationConfig, FeishuChannelConfig, MemoryConfig, ProviderConfig, ProviderKind,
        ReasoningEffort, ToolConfig,
    };
    use serde_json::json;

    #[test]
    fn message_builder_includes_system_prompt() {
        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        assert!(!messages.is_empty());
        assert_eq!(messages[0]["role"], "system");
    }

    #[test]
    fn build_messages_includes_capability_snapshot_block() {
        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        assert!(!messages.is_empty());
        let system_content = messages[0]["content"].as_str().expect("system content");
        assert!(
            system_content.contains("[available_tools]"),
            "system prompt should contain capability snapshot marker, got: {system_content}"
        );
        assert!(
            system_content.contains("- shell.exec: Execute shell commands"),
            "system prompt should list shell.exec tool"
        );
        assert!(
            system_content.contains("- file.read: Read file contents"),
            "system prompt should list file.read tool"
        );
        assert!(
            system_content.contains("- file.write: Write file contents"),
            "system prompt should list file.write tool"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn build_messages_skips_internal_conversation_events_in_history_window() {
        let mut config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let session_id = format!(
            "provider-history-filter-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        config.memory.sqlite_path = std::env::temp_dir()
            .join(format!("{session_id}.sqlite3"))
            .display()
            .to_string();
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
            sliding_window: Some(config.memory.sliding_window),
        };
        crate::memory::append_turn_direct(&session_id, "user", "hello", &memory_config)
            .expect("persist user turn");
        crate::memory::append_turn_direct(
            &session_id,
            "assistant",
            r#"{"type":"conversation_event","event":"lane_selected","payload":{"lane":"safe"}}"#,
            &memory_config,
        )
        .expect("persist conversation event");
        crate::memory::append_turn_direct(
            &session_id,
            "assistant",
            r#"{"type":"tool_outcome","turn_id":"t1","tool_call_id":"c1","outcome":{"status":"ok"}}"#,
            &memory_config,
        )
        .expect("persist tool outcome");
        crate::memory::append_turn_direct(
            &session_id,
            "assistant",
            "normal assistant reply",
            &memory_config,
        )
        .expect("persist assistant reply");

        let messages =
            build_messages_for_session(&config, &session_id, true).expect("build messages");
        let history_contents = messages
            .iter()
            .skip(1)
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(
            history_contents.contains(&"hello"),
            "expected user content in history: {history_contents:?}"
        );
        assert!(
            history_contents.contains(&"normal assistant reply"),
            "expected normal assistant content in history: {history_contents:?}"
        );
        assert!(
            history_contents
                .iter()
                .all(|content| !content.contains("\"type\":\"conversation_event\"")),
            "conversation_event payload must be filtered out: {history_contents:?}"
        );
        assert!(
            history_contents
                .iter()
                .all(|content| !content.contains("\"type\":\"tool_outcome\"")),
            "tool_outcome payload must be filtered out: {history_contents:?}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn build_messages_skips_unknown_history_roles() {
        let mut config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let session_id = format!(
            "provider-history-role-filter-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        config.memory.sqlite_path = std::env::temp_dir()
            .join(format!("{session_id}.sqlite3"))
            .display()
            .to_string();
        let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
            sliding_window: Some(config.memory.sliding_window),
        };
        crate::memory::append_turn_direct(&session_id, "user", "hello", &memory_config)
            .expect("persist user turn");
        crate::memory::append_turn_direct(
            &session_id,
            "internal_event",
            "should be hidden",
            &memory_config,
        )
        .expect("persist unknown role turn");
        crate::memory::append_turn_direct(
            &session_id,
            "assistant",
            "visible reply",
            &memory_config,
        )
        .expect("persist assistant turn");

        let messages =
            build_messages_for_session(&config, &session_id, true).expect("build messages");
        let history_roles = messages
            .iter()
            .skip(1)
            .filter_map(|message| message.get("role").and_then(Value::as_str))
            .collect::<Vec<_>>();
        let history_contents = messages
            .iter()
            .skip(1)
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(
            history_roles.iter().all(|role| *role != "internal_event"),
            "unknown roles should be filtered: {history_roles:?}"
        );
        assert!(
            history_contents
                .iter()
                .all(|content| *content != "should be hidden"),
            "unknown role content should not be included: {history_contents:?}"
        );
        assert!(
            history_contents.contains(&"visible reply"),
            "assistant content should still be kept: {history_contents:?}"
        );
    }

    #[test]
    fn completion_body_includes_reasoning_effort_when_configured() {
        let mut config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
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
    fn kimi_coding_completion_body_adds_extra_body_thinking() {
        let mut config = LoongClawConfig {
            provider: ProviderConfig {
                kind: ProviderKind::KimiCoding,
                ..ProviderConfig::default()
            },
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
            conversation: crate::config::ConversationConfig::default(),
        };
        config.provider.reasoning_effort = Some(ReasoningEffort::High);

        let body = build_completion_request_body(
            &config,
            &[],
            "kimi-for-coding",
            CompletionPayloadMode::default_for(&config.provider),
        );
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["extra_body"]["thinking"]["type"], "enabled");
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
            conversation: ConversationConfig::default(),
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
    fn kimi_coding_request_headers_include_default_user_agent() {
        let provider = ProviderConfig {
            kind: ProviderKind::KimiCoding,
            ..ProviderConfig::default()
        };
        let headers = transport::build_request_headers(&provider).expect("headers");
        let user_agent = headers
            .get(reqwest::header::USER_AGENT)
            .expect("default user-agent")
            .to_str()
            .expect("user-agent value");
        assert_eq!(user_agent, "KimiCLI/LoongClaw");
    }

    #[test]
    fn kimi_coding_keeps_explicit_compatible_user_agent() {
        let provider = ProviderConfig {
            kind: ProviderKind::KimiCoding,
            headers: [("User-Agent".to_owned(), "KimiCLI/custom".to_owned())]
                .into_iter()
                .collect(),
            ..ProviderConfig::default()
        };
        let headers = transport::build_request_headers(&provider).expect("headers");
        let user_agent = headers
            .get(reqwest::header::USER_AGENT)
            .expect("explicit user-agent")
            .to_str()
            .expect("user-agent value");
        assert_eq!(user_agent, "KimiCLI/custom");
    }

    #[cfg(any(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn turn_body_includes_tool_schema_and_auto_choice() {
        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
        };

        let body = build_turn_request_body(
            &config,
            &[],
            "model-latest",
            CompletionPayloadMode::default_for(&config.provider),
            true,
            &crate::tools::provider_tool_definitions(),
        );
        let tools = body
            .get("tools")
            .and_then(|value| value.as_array())
            .expect("tools array in turn body");
        assert!(!tools.is_empty());
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();

        let mut expected = Vec::new();
        #[cfg(feature = "tool-file")]
        {
            expected.push("file_read");
            expected.push("file_write");
        }
        #[cfg(feature = "tool-shell")]
        {
            expected.push("shell_exec");
        }

        assert_eq!(names, expected);
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn tool_schema_fallback_detects_unsupported_error_shapes() {
        let unsupported_tools = json!({
            "error": {
                "code": "unsupported_parameter",
                "param": "tools",
                "message": "Unsupported parameter: tools"
            }
        });
        let unsupported_tool_choice = json!({
            "error": {
                "message": "Function calling is not supported for this model."
            }
        });

        assert!(should_disable_tool_schema_for_error(
            &parse_provider_api_error(&unsupported_tools)
        ));
        assert!(should_disable_tool_schema_for_error(
            &parse_provider_api_error(&unsupported_tool_choice)
        ));
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

    #[test]
    fn validate_provider_configuration_rejects_plain_kimi_on_coding_endpoint() {
        let config = LoongClawConfig {
            provider: ProviderConfig {
                kind: ProviderKind::Kimi,
                base_url: "https://api.kimi.com/coding".to_owned(),
                chat_completions_path: "/v1/chat/completions".to_owned(),
                ..ProviderConfig::default()
            },
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
            conversation: crate::config::ConversationConfig::default(),
        };
        let error = validate_provider_configuration(&config).expect_err("misconfig should fail");
        assert!(error.contains("use `kind = \"kimi_coding\"`"));
    }

    #[test]
    fn validate_provider_configuration_rejects_incompatible_kimi_coding_user_agent() {
        let config = LoongClawConfig {
            provider: ProviderConfig {
                kind: ProviderKind::KimiCoding,
                headers: [("User-Agent".to_owned(), "LoongClaw/0.1".to_owned())]
                    .into_iter()
                    .collect(),
                ..ProviderConfig::default()
            },
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            tools: ToolConfig::default(),
            memory: MemoryConfig::default(),
            conversation: crate::config::ConversationConfig::default(),
        };
        let error = validate_provider_configuration(&config).expect_err("invalid ua");
        assert!(error.contains("KimiCLI/"));
    }
}
