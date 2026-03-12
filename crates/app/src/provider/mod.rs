use std::sync::atomic::{AtomicBool, Ordering};
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
    CompletionPayloadMode, ProviderApiError, adapt_payload_mode_for_error,
    build_completion_request_body, build_turn_request_body, parse_provider_api_error,
    should_disable_tool_schema_for_error, should_try_next_model_on_error,
};

pub use shape::extract_provider_turn;

pub fn build_system_message(
    config: &LoongClawConfig,
    include_system_prompt: bool,
) -> Option<Value> {
    build_system_message_with_tool_runtime_config(
        config,
        include_system_prompt,
        crate::tools::runtime_config::get_tool_runtime_config(),
    )
}

fn build_system_message_with_tool_runtime_config(
    config: &LoongClawConfig,
    include_system_prompt: bool,
    tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Option<Value> {
    if !include_system_prompt {
        return None;
    }
    let system_prompt = config.cli.resolved_system_prompt();
    let system = system_prompt.trim();
    let snapshot = super::tools::capability_snapshot_with_config(tool_runtime_config);
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

#[cfg(feature = "memory-sqlite")]
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

#[cfg(feature = "memory-sqlite")]
fn is_supported_chat_role(role: &str) -> bool {
    matches!(role, "system" | "user" | "assistant" | "tool")
}

#[cfg(feature = "memory-sqlite")]
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
        let mem_config =
            super::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let memory_entries = memory::load_prompt_context(session_id, &mem_config)
            .map_err(|error| format!("load prompt memory context failed: {error}"))?;
        let mut messages = Vec::with_capacity(memory_entries.len());
        for entry in memory_entries {
            match entry.kind {
                memory::MemoryContextKind::Profile | memory::MemoryContextKind::Summary => {
                    messages.push(json!({
                        "role": entry.role,
                        "content": entry.content,
                    }));
                }
                memory::MemoryContextKind::Turn => {
                    push_history_message(
                        &mut messages,
                        entry.role.as_str(),
                        entry.content.as_str(),
                    );
                }
            }
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

/// Caller hook signal: `Retry` = handled, loop again; `Continue` = fall through.
enum ErrorHookAction {
    Retry,
    Continue,
}

/// Generic retry loop shared by completion and turn request paths.
async fn execute_with_retry<T>(
    config: &LoongClawConfig,
    model: &str,
    request_context: &ProviderRequestContext<'_>,
    mut build_body: impl FnMut(CompletionPayloadMode) -> Value,
    extract_result: impl Fn(&Value, usize) -> Result<T, ModelRequestError>,
    mut on_error_hook: impl FnMut(&ProviderApiError) -> ErrorHookAction,
) -> Result<T, ModelRequestError> {
    let mut attempt = 0usize;
    let mut backoff_ms = request_context.request_policy.initial_backoff_ms;
    let mut payload_mode = CompletionPayloadMode::default_for(&config.provider);
    let mut tried_payload_modes = vec![payload_mode];
    let max_attempts = request_context.request_policy.max_attempts;

    loop {
        attempt += 1;
        let body = build_body(payload_mode);
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
                        ),
                        try_next_model: false,
                    })?;

                if status.is_success() {
                    return extract_result(&response_body, attempt);
                }

                let api_error = parse_provider_api_error(&response_body);

                // Caller-specific error hook (e.g. tool-schema disable for turn path).
                if matches!(on_error_hook(&api_error), ErrorHookAction::Retry) {
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
                if attempt < max_attempts && policy::should_retry_status(status_code) {
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
                    ),
                    try_next_model: false,
                });
            }
            Err(error) => {
                if attempt < max_attempts && policy::should_retry_error(&error) {
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
                    ),
                    try_next_model: false,
                });
            }
        }
    }
}

async fn request_completion_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    request_context: &ProviderRequestContext<'_>,
) -> Result<String, ModelRequestError> {
    let max_attempts = request_context.request_policy.max_attempts;
    execute_with_retry(
        config,
        model,
        request_context,
        |payload_mode| build_completion_request_body(config, messages, model, payload_mode),
        |response_body, attempt| {
            shape::extract_message_content(response_body).ok_or_else(|| ModelRequestError {
                message: format!(
                    "provider response missing choices[0].message.content for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                ),
                try_next_model: false,
            })
        },
        |_api_error: &ProviderApiError| ErrorHookAction::Continue,
    )
    .await
}

async fn request_turn_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: &str,
    request_context: &ProviderRequestContext<'_>,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let max_attempts = request_context.request_policy.max_attempts;
    let tool_definitions = super::tools::provider_tool_definitions();
    let include_tool_schema = AtomicBool::new(!tool_definitions.is_empty());

    execute_with_retry(
        config,
        model,
        request_context,
        |payload_mode| {
            build_turn_request_body(
                config,
                messages,
                model,
                payload_mode,
                include_tool_schema.load(Ordering::Relaxed),
                &tool_definitions,
            )
        },
        |response_body, attempt| {
            shape::extract_provider_turn(response_body).ok_or_else(|| ModelRequestError {
                message: format!(
                    "provider response missing choices[0].message for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                ),
                try_next_model: false,
            })
        },
        |api_error| {
            if include_tool_schema.load(Ordering::Relaxed)
                && should_disable_tool_schema_for_error(api_error)
            {
                include_tool_schema.store(false, Ordering::Relaxed);
                ErrorHookAction::Retry
            } else {
                ErrorHookAction::Continue
            }
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::model_selection::rank_model_candidates;
    use super::payload_adaptation::{ReasoningField, TemperatureField, TokenLimitField};
    use super::*;
    use crate::config::{
        ConversationConfig, ExternalSkillsConfig, FeishuChannelConfig, MemoryConfig,
        ProviderConfig, ProviderKind, ReasoningEffort, ToolConfig,
    };
    use serde_json::json;

    static PROVIDER_TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn write_provider_test_file(root: &std::path::Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }
        std::fs::write(path, content).expect("write fixture");
    }

    fn install_skill_for_provider_snapshot_test(
        auto_expose_installed: bool,
    ) -> (
        LoongClawConfig,
        crate::tools::runtime_config::ToolRuntimeConfig,
        std::path::PathBuf,
    ) {
        let sequence = PROVIDER_TEST_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "loongclaw-provider-ext-skills-{}-{sequence}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        write_provider_test_file(
            &root,
            "skills/demo-skill/SKILL.md",
            "# Demo Skill\n\nUse this skill for provider prompt verification.\n",
        );

        let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
            shell_allowlist: std::collections::BTreeSet::new(),
            file_root: Some(root.clone()),
            external_skills: crate::tools::runtime_config::ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: std::collections::BTreeSet::new(),
                blocked_domains: std::collections::BTreeSet::new(),
                install_root: None,
                auto_expose_installed,
            },
        };
        crate::tools::execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "skills/demo-skill"
                }),
            },
            &tool_runtime_config,
        )
        .expect("install should succeed");

        let config = LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            external_skills: ExternalSkillsConfig::default(),
            memory: MemoryConfig::default(),
            acp: crate::config::AcpConfig::default(),
        };
        (config, tool_runtime_config, root)
    }

    fn default_test_config() -> LoongClawConfig {
        LoongClawConfig {
            provider: ProviderConfig::default(),
            cli: crate::config::CliChannelConfig::default(),
            telegram: crate::config::TelegramChannelConfig::default(),
            feishu: FeishuChannelConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolConfig::default(),
            external_skills: ExternalSkillsConfig::default(),
            memory: MemoryConfig::default(),
            acp: crate::config::AcpConfig::default(),
        }
    }

    #[test]
    fn message_builder_includes_system_prompt() {
        let config = default_test_config();

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        assert!(!messages.is_empty());
        assert_eq!(messages[0]["role"], "system");
    }

    #[test]
    fn build_messages_includes_capability_snapshot_block() {
        let config = default_test_config();

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

    #[test]
    fn build_system_message_can_include_installed_external_skills_snapshot() {
        let (config, tool_runtime_config, root) = install_skill_for_provider_snapshot_test(true);

        let system_message =
            build_system_message_with_tool_runtime_config(&config, true, &tool_runtime_config)
                .expect("system message");
        let system_content = system_message["content"].as_str().expect("system content");

        assert!(system_content.contains("[available_external_skills]"));
        assert!(
            system_content.contains(
                "- demo-skill: installed managed external skill; use external_skills.inspect or external_skills.invoke for details"
            )
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn build_system_message_omits_installed_external_skills_when_auto_expose_is_disabled() {
        let (config, tool_runtime_config, root) = install_skill_for_provider_snapshot_test(false);

        let system_message =
            build_system_message_with_tool_runtime_config(&config, true, &tool_runtime_config)
                .expect("system message");
        let system_content = system_message["content"].as_str().expect("system content");

        assert!(!system_content.contains("[available_external_skills]"));
        assert!(!system_content.contains("demo-skill"));

        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn build_messages_skips_internal_conversation_events_in_history_window() {
        let mut config = default_test_config();

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
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
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
        let mut config = default_test_config();

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
        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
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
    fn message_builder_uses_rendered_prompt_from_pack_metadata() {
        let mut config = default_test_config();
        config.cli.personality = Some(crate::prompt::PromptPersonality::FriendlyCollab);
        config.cli.system_prompt = String::new();

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        let system_content = messages[0]["content"].as_str().expect("system content");

        assert!(system_content.contains("## Personality Overlay: Friendly Collaboration"));
        assert!(system_content.contains("[available_tools]"));
    }

    #[test]
    fn message_builder_keeps_legacy_inline_prompt_when_pack_is_disabled() {
        let mut config = default_test_config();
        config.cli.prompt_pack_id = None;
        config.cli.personality = None;
        config.cli.system_prompt = "You are a legacy inline prompt.".to_owned();

        let messages =
            build_messages_for_session(&config, "noop-session", true).expect("build messages");
        let system_content = messages[0]["content"].as_str().expect("system content");

        assert!(system_content.contains("You are a legacy inline prompt."));
        assert!(!system_content.contains("## Personality Overlay: Calm Engineering"));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn message_builder_includes_summary_block_for_window_plus_summary_profile() {
        let tmp =
            std::env::temp_dir().join(format!("loongclaw-provider-summary-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("provider-summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = default_test_config();
        config.memory.sqlite_path = db_path.display().to_string();
        config.memory.profile = crate::config::MemoryProfile::WindowPlusSummary;
        config.memory.sliding_window = 2;

        let memory_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        crate::memory::append_turn_direct("summary-session", "user", "turn 1", &memory_config)
            .expect("append turn 1 should succeed");
        crate::memory::append_turn_direct("summary-session", "assistant", "turn 2", &memory_config)
            .expect("append turn 2 should succeed");
        crate::memory::append_turn_direct("summary-session", "user", "turn 3", &memory_config)
            .expect("append turn 3 should succeed");
        crate::memory::append_turn_direct("summary-session", "assistant", "turn 4", &memory_config)
            .expect("append turn 4 should succeed");

        let messages =
            build_messages_for_session(&config, "summary-session", true).expect("build messages");

        assert!(
            messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("## Memory Summary"))
            }),
            "expected a system summary block in provider messages"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[test]
    fn completion_body_includes_reasoning_effort_when_configured() {
        let mut config = default_test_config();
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
        let mut config = default_test_config();
        config.provider.kind = ProviderKind::KimiCoding;
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
        let config = default_test_config();

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
        let config = default_test_config();

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

        let mut expected = vec![
            "claw_import",
            "external_skills_fetch",
            "external_skills_inspect",
            "external_skills_install",
            "external_skills_invoke",
            "external_skills_list",
            "external_skills_policy",
            "external_skills_remove",
        ];
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
        let mut config = default_test_config();
        config.provider.kind = ProviderKind::Kimi;
        config.provider.base_url = "https://api.kimi.com/coding".to_owned();
        config.provider.chat_completions_path = "/v1/chat/completions".to_owned();
        let error = validate_provider_configuration(&config).expect_err("misconfig should fail");
        assert!(error.contains("use `kind = \"kimi_coding\"`"));
    }

    #[test]
    fn validate_provider_configuration_rejects_incompatible_kimi_coding_user_agent() {
        let mut config = default_test_config();
        config.provider.kind = ProviderKind::KimiCoding;
        config.provider.headers = [("User-Agent".to_owned(), "LoongClaw/0.1".to_owned())]
            .into_iter()
            .collect();
        let error = validate_provider_configuration(&config).expect_err("invalid ua");
        assert!(error.contains("KimiCLI/"));
    }
}
