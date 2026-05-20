use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(test)]
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
#[cfg(not(test))]
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::control::{GatewayControlAppState, authorize_request_from_state};
use crate::mvp::config::{LoongConfig, ProviderProfileConfig};
use crate::task_execution::execute_daemon_turn_gateway_request;

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatCompletionMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    stop: Option<Value>,
    #[serde(default)]
    tools: Option<Value>,
    #[serde(default)]
    tool_choice: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    role: String,
    content: Value,
}

#[derive(Debug, Serialize)]
struct ModelListResponse {
    object: &'static str,
    data: Vec<ModelObject>,
}

#[derive(Debug, Serialize)]
struct ModelObject {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: String,
}

#[derive(Clone)]
struct ConfiguredModelBinding {
    request_model_id: String,
    profile_id: String,
    owned_by: String,
    provider: crate::mvp::config::ProviderConfig,
}

struct OpenAiCompatGatewayTurnSeed {
    request_id: String,
    session_id: String,
    model: String,
    run_config: LoongConfig,
    input: String,
    resolved_path: Option<std::path::PathBuf>,
}

struct OpenAiCompatStreamObserver {
    sender: mpsc::UnboundedSender<Result<Event, Infallible>>,
    request_id: String,
    model: String,
    emitted_text: AtomicBool,
}

impl OpenAiCompatStreamObserver {
    fn new(
        sender: mpsc::UnboundedSender<Result<Event, Infallible>>,
        request_id: String,
        model: String,
    ) -> Self {
        Self {
            sender,
            request_id,
            model,
            emitted_text: AtomicBool::new(false),
        }
    }

    fn emitted_text(&self) -> bool {
        self.emitted_text.load(Ordering::Relaxed)
    }

    fn push_text(&self, text: &str) {
        self.emitted_text.store(true, Ordering::Relaxed);
        let _ = self.sender.send(Ok(build_sse_event(build_content_chunk(
            self.request_id.as_str(),
            self.model.as_str(),
            text,
        ))));
    }
}

impl crate::mvp::conversation::ConversationTurnObserver for OpenAiCompatStreamObserver {
    fn on_streaming_token(&self, event: crate::mvp::acp::StreamingTokenEvent) {
        if event.event_type != "text_delta" {
            return;
        }
        let Some(text) = event.delta.text.as_deref() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.push_text(text);
    }
}

static OPENAI_COMPAT_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static OPENAI_COMPAT_TEST_ARTIFACT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn handle_models(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": error}));
    }

    let Some(config) = app_state.config.as_ref() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "gateway config not available"}),
        );
    };

    let payload = ModelListResponse {
        object: "list",
        data: configured_openai_models(config),
    };
    match serde_json::to_value(payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": format!("response serialization failed: {error}")}),
        ),
    }
}

pub(crate) async fn handle_chat_completions(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Response {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": error}));
    }

    let Some(config) = app_state.config.as_ref() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "gateway config not available"}),
        );
    };

    if request.tools.is_some() || request.tool_choice.is_some() {
        let unsupported_param = if request.tools.is_some() {
            "tools"
        } else {
            "tool_choice"
        };
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({
                "error": {
                    "message": "tools and tool_choice are not supported on this OpenAI-compatible gateway surface yet",
                    "param": unsupported_param
                }
            }),
        );
    }

    if request.messages.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": "messages must not be empty", "param": "messages"}}),
        );
    }
    if let Err(error) = map_chat_completion_messages(&request.messages) {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": error, "param": "messages"}}),
        );
    }
    if let Some(stop) = &request.stop
        && let Err(error) = parse_stop_sequences(stop)
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": error, "param": "stop"}}),
        );
    }

    if resolve_model_binding(config, request.model.as_str()).is_none() {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({"error": {"message": format!("unknown model `{}`", request.model), "param": "model"}}),
        );
    }
    if let Err(error) = validate_gateway_turn_request_shape(&request) {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": {"message": error, "param": "messages"}}),
        );
    }
    if request.stream {
        return stream_chat_completion(app_state.as_ref(), config, &request).await;
    }

    complete_chat_completion(app_state.as_ref(), config, &request)
        .await
        .map_or_else(
            |error| {
                json_response(
                    gateway_runtime_error_status(error.as_str()),
                    json!({"error": {"message": error}}),
                )
            },
            |payload| json_response(StatusCode::OK, payload),
        )
}

fn configured_openai_models(config: &LoongConfig) -> Vec<ModelObject> {
    configured_model_bindings(config)
        .into_iter()
        .map(|binding| ModelObject {
            id: binding.request_model_id,
            object: "model",
            created: 0,
            owned_by: binding.owned_by,
        })
        .collect()
}

fn resolve_model_binding(config: &LoongConfig, model: &str) -> Option<ConfiguredModelBinding> {
    configured_model_bindings(config)
        .into_iter()
        .find(|binding| binding.request_model_id == model)
}

fn configured_provider_profiles(config: &LoongConfig) -> Vec<(String, ProviderProfileConfig)> {
    if config.providers.is_empty() {
        return vec![(
            config
                .active_provider_id()
                .unwrap_or(config.provider.kind.profile().id)
                .to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: config.provider.clone(),
            },
        )];
    }
    config
        .providers
        .iter()
        .map(|(profile_id, profile)| (profile_id.clone(), profile.clone()))
        .collect()
}

fn configured_model_bindings(config: &LoongConfig) -> Vec<ConfiguredModelBinding> {
    let provider_profiles = configured_provider_profiles(config);
    let mut raw_model_counts = std::collections::BTreeMap::new();
    for (_profile_id, profile) in &provider_profiles {
        for model_id in configured_provider_model_ids(&profile.provider) {
            *raw_model_counts.entry(model_id).or_insert(0usize) += 1;
        }
    }
    let mut bindings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (profile_id, profile) in provider_profiles {
        let provider = profile.provider;
        for provider_model_id in configured_provider_model_ids(&provider) {
            let duplicate_count = raw_model_counts
                .get(&provider_model_id)
                .copied()
                .unwrap_or_default();
            let request_model_id = if duplicate_count > 1 {
                format!("{profile_id}:{provider_model_id}")
            } else {
                provider_model_id.clone()
            };
            if !seen.insert(request_model_id.clone()) {
                continue;
            }
            let mut bound_provider = provider.clone();
            bound_provider.model = provider_model_id.clone();
            bindings.push(ConfiguredModelBinding {
                request_model_id,
                profile_id: profile_id.clone(),
                owned_by: provider.kind.as_str().to_owned(),
                provider: bound_provider,
            });
        }
    }
    bindings
}

fn configured_provider_model_ids(provider: &crate::mvp::config::ProviderConfig) -> Vec<String> {
    if let Some(explicit_model) = provider.explicit_model() {
        return vec![explicit_model];
    }
    if !provider.configured_auto_model_candidates().is_empty() {
        return provider.configured_auto_model_candidates();
    }
    vec![provider.configured_model_value()]
}

fn configured_provider_for_request(
    config: &LoongConfig,
    request: &ChatCompletionRequest,
) -> Result<ConfiguredModelBinding, String> {
    let mut binding = resolve_model_binding(config, request.model.as_str())
        .ok_or_else(|| format!("unknown model `{}`", request.model))?;
    if let Some(temperature) = request.temperature {
        binding.provider.temperature = temperature;
    }
    if let Some(max_tokens) = request.max_tokens {
        binding.provider.max_tokens = Some(max_tokens);
    }
    if let Some(stop) = &request.stop {
        binding.provider.stop = parse_stop_sequences(stop)?;
    }
    Ok(binding)
}

fn validate_gateway_turn_request_shape(request: &ChatCompletionRequest) -> Result<(), String> {
    let Some(last_message) = request.messages.last() else {
        return Err("messages must not be empty".to_owned());
    };
    if last_message.role.trim() != "user" {
        return Err("messages must end with a `user` role for this gateway surface".to_owned());
    }
    Ok(())
}

fn next_openai_compat_request_id(model: &str) -> String {
    let counter = OPENAI_COMPAT_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("chatcmpl-openai-compat-{model}-{counter}")
}

fn parse_stop_sequences(raw: &Value) -> Result<Vec<String>, String> {
    if let Some(value) = raw.as_str() {
        return Ok(vec![value.to_owned()]);
    }
    let Some(items) = raw.as_array() else {
        return Err("stop must be a string or array of strings".to_owned());
    };
    let mut values = Vec::new();
    for item in items {
        let Some(value) = item.as_str() else {
            return Err("stop array entries must be strings".to_owned());
        };
        values.push(value.to_owned());
    }
    Ok(values)
}

fn map_chat_completion_messages(messages: &[ChatCompletionMessage]) -> Result<Vec<Value>, String> {
    messages
        .iter()
        .map(|message| {
            let role = message.role.trim();
            if !matches!(role, "system" | "user" | "assistant") {
                return Err(format!("unsupported message role `{role}`"));
            }
            let content = render_message_content(&message.content)?;
            Ok(json!({
                "role": role,
                "content": content,
            }))
        })
        .collect()
}

fn render_message_content(content: &Value) -> Result<String, String> {
    if let Some(text) = content.as_str() {
        return Ok(text.to_owned());
    }
    if let Some(parts) = content.as_array() {
        let mut text = String::new();
        for part in parts {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
            if part_type != "text" {
                return Err(format!("unsupported content part type `{part_type}`"));
            }
            let Some(part_text) = part.get("text").and_then(Value::as_str) else {
                return Err("text content part is missing `text`".to_owned());
            };
            text.push_str(part_text);
        }
        return Ok(text);
    }
    Err("unsupported message content shape".to_owned())
}

fn chat_message_to_window_turn(
    message: &ChatCompletionMessage,
    ts: i64,
) -> Result<crate::mvp::memory::WindowTurn, String> {
    Ok(crate::mvp::memory::WindowTurn {
        role: message.role.trim().to_owned(),
        content: render_message_content(&message.content)?,
        ts: Some(ts),
    })
}

fn build_gateway_turn_seed(
    config: &LoongConfig,
    request: &ChatCompletionRequest,
) -> Result<OpenAiCompatGatewayTurnSeed, String> {
    let Some((last_message, history)) = request.messages.split_last() else {
        return Err("messages must not be empty".to_owned());
    };
    if last_message.role.trim() != "user" {
        return Err("messages must end with a `user` role for this gateway surface".to_owned());
    }

    let binding = configured_provider_for_request(config, request)?;
    let input = render_message_content(&last_message.content)?;
    let history_turns = history
        .iter()
        .enumerate()
        .map(|(index, message)| chat_message_to_window_turn(message, index as i64))
        .collect::<Result<Vec<_>, _>>()?;
    let request_id = next_openai_compat_request_id(request.model.as_str());
    let mut run_config = config.clone();
    let bound_profile_id = binding.profile_id.clone();
    let bound_provider = binding.provider;
    run_config.provider = bound_provider.clone();
    run_config.providers = BTreeMap::from([(
        bound_profile_id.clone(),
        ProviderProfileConfig {
            default_for_kind: true,
            provider: bound_provider,
        },
    )]);
    run_config.active_provider = Some(bound_profile_id);
    run_config.last_provider = None;
    let memory_config = crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
        &run_config.memory,
    );
    crate::mvp::memory::execute_memory_core_with_config(
        crate::mvp::memory::build_replace_turns_request(request_id.as_str(), &history_turns),
        &memory_config,
    )
    .map_err(|error| format!("seed gateway turn session failed: {error}"))?;

    #[cfg(test)]
    let resolved_path = Some(persist_openai_compat_turn_runtime_config(&run_config)?);
    #[cfg(not(test))]
    let resolved_path = None;

    Ok(OpenAiCompatGatewayTurnSeed {
        request_id: request_id.clone(),
        session_id: request_id,
        model: request.model.clone(),
        run_config,
        input,
        resolved_path,
    })
}

async fn run_gateway_turn_for_seed(
    resolved_path: std::path::PathBuf,
    seed: &OpenAiCompatGatewayTurnSeed,
    observer: Option<crate::mvp::conversation::ConversationTurnObserverHandle>,
) -> Result<crate::mvp::agent_runtime::AgentTurnResult, String> {
    let request = loong_app::turn_gateway::build_turn_gateway_request(
        loong_app::conversation::ConversationSessionAddress::from_session_id(
            seed.session_id.as_str(),
        ),
        seed.input.clone(),
        BTreeMap::new(),
        crate::mvp::agent_runtime::AgentTurnMode::Oneshot,
        crate::mvp::acp::AcpRoutingIntent::Automatic,
        false,
        Vec::new(),
        None,
        false,
    );
    let turn_service = crate::mvp::agent_runtime::TurnExecutionService::new(
        resolved_path,
        seed.run_config.clone(),
    )
    .without_runtime_environment_init();
    execute_daemon_turn_gateway_request(
        &turn_service,
        Some(seed.session_id.as_str()),
        request,
        observer,
        crate::mvp::conversation::ProviderErrorMode::Propagate,
    )
    .await
}

async fn complete_chat_completion(
    app_state: &GatewayControlAppState,
    config: &LoongConfig,
    request: &ChatCompletionRequest,
) -> Result<Value, String> {
    let seed = build_gateway_turn_seed(config, request)?;
    let resolved_path = seed
        .resolved_path
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(app_state.config_path.clone()));
    let result = run_gateway_turn_for_seed(resolved_path, &seed, None).await?;
    Ok(json!({
        "id": seed.request_id,
        "object": "chat.completion",
        "created": 0,
        "model": seed.model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": result.output_text,
            },
            "finish_reason": "stop",
        }],
        "usage": result.usage.unwrap_or(Value::Null),
    }))
}

async fn stream_chat_completion(
    app_state: &GatewayControlAppState,
    config: &LoongConfig,
    request: &ChatCompletionRequest,
) -> Response {
    let seed = match build_gateway_turn_seed(config, request) {
        Ok(seed) => seed,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": error, "param": "messages"}}),
            );
        }
    };
    if !crate::mvp::provider::supports_turn_streaming_events(&seed.run_config) {
        return json_response(
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": {"message": format!("model `{}` does not support live streaming events", request.model), "param": "model"}}),
        );
    }
    let (sender, receiver) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let observer = Arc::new(OpenAiCompatStreamObserver::new(
        sender.clone(),
        seed.request_id.clone(),
        seed.model.clone(),
    ));
    let observer_handle: crate::mvp::conversation::ConversationTurnObserverHandle =
        observer.clone();
    let request_id = seed.request_id.clone();
    let model = seed.model.clone();
    let resolved_path = seed
        .resolved_path
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(app_state.config_path.clone()));

    tokio::spawn(async move {
        let result = run_gateway_turn_for_seed(resolved_path, &seed, Some(observer_handle)).await;
        match result {
            Ok(result) => {
                if !observer.emitted_text() && !result.output_text.is_empty() {
                    observer.push_text(result.output_text.as_str());
                }
                let _ = sender.send(Ok(build_sse_event(build_finish_chunk(
                    request_id.as_str(),
                    model.as_str(),
                    "stop",
                ))));
                let _ = sender.send(Ok(Event::default().data("[DONE]")));
            }
            Err(error) => {
                let _ = sender.send(Ok(build_sse_event(json!({
                    "error": {
                        "message": error
                    }
                }))));
                let _ = sender.send(Ok(Event::default().data("[DONE]")));
            }
        }
    });

    let sse_stream = stream::unfold(receiver, |mut receiver| async {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Sse::new(sse_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn build_content_chunk(request_id: &str, model: &str, content: &str) -> Value {
    json!({
        "id": request_id,
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant",
                "content": content,
            },
            "finish_reason": Value::Null,
        }],
    })
}

fn build_finish_chunk(request_id: &str, model: &str, finish_reason: &str) -> Value {
    json!({
        "id": request_id,
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": finish_reason,
        }],
    })
}

fn build_sse_event(payload: Value) -> Event {
    Event::default().data(payload.to_string())
}

fn gateway_runtime_error_status(error: &str) -> StatusCode {
    crate::mvp::provider::parse_provider_failover_snapshot_payload(error)
        .and_then(|payload| payload.get("status_code").and_then(Value::as_u64))
        .and_then(|status| u16::try_from(status).ok())
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn json_response(status: StatusCode, payload: Value) -> Response {
    (status, Json(payload)).into_response()
}

#[cfg(test)]
#[doc(hidden)]
pub fn build_openai_compat_test_router_no_backend(
    config: LoongConfig,
    bearer_token: String,
) -> Router {
    let mut app_state = GatewayControlAppState::test_minimal(bearer_token);
    let (runtime_dir, config_path) = prepare_openai_compat_test_runtime(config.clone());
    crate::mvp::runtime_env::initialize_runtime_environment(
        &config,
        Some(std::path::Path::new(config_path.as_str())),
    );
    app_state.runtime_dir = runtime_dir;
    app_state.config_path = config_path;
    app_state.config = Some(config);
    build_openai_compat_router(Arc::new(app_state))
}

#[cfg(test)]
fn prepare_openai_compat_test_runtime(config: LoongConfig) -> (std::path::PathBuf, String) {
    let root = std::env::temp_dir().join(next_openai_compat_test_artifact_name("runtime"));
    std::fs::create_dir_all(&root).expect("create openai compat runtime root");

    let config_path = root.join("loong.toml");
    let config_path_text = config_path.display().to_string();
    crate::mvp::config::write(Some(config_path_text.as_str()), &config, true)
        .expect("write openai compat test config");

    let session_store_config =
        crate::mvp::session::store::session_store_config_from_memory_config_without_env_overrides(
            &config.memory,
        );
    crate::mvp::session::store::ensure_session_store_ready(
        Some(config.memory.resolved_sqlite_path()),
        &session_store_config,
    )
    .expect("initialize openai compat session store");

    (root, config_path_text)
}

#[cfg(test)]
fn persist_openai_compat_turn_runtime_config(
    config: &LoongConfig,
) -> Result<std::path::PathBuf, String> {
    let root = std::env::temp_dir().join(next_openai_compat_test_artifact_name("turn"));
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create openai compat turn config dir failed: {error}"))?;

    let config_path = root.join("loong.toml");
    let config_path_text = config_path.display().to_string();
    crate::mvp::config::write(Some(config_path_text.as_str()), config, true)
        .map_err(|error| format!("write openai compat turn config failed: {error}"))?;

    Ok(config_path)
}

#[cfg(test)]
fn next_openai_compat_test_artifact_name(label: &str) -> String {
    let counter = OPENAI_COMPAT_TEST_ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();
    format!("loong-openai-compat-{label}-{process_id}-{counter}")
}

#[cfg(test)]
pub(crate) fn build_openai_compat_router(app_state: Arc<GatewayControlAppState>) -> Router {
    Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .with_state(app_state)
}

#[cfg(test)]
#[path = "openai_compat_tests.rs"]
mod openai_compat_tests;
