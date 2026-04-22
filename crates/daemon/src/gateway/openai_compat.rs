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

fn build_openai_compat_turn_request(input: String) -> crate::mvp::agent_runtime::AgentTurnRequest {
    crate::mvp::agent_runtime::AgentTurnRequest {
        message: input,
        turn_mode: crate::mvp::agent_runtime::AgentTurnMode::Oneshot,
        ..Default::default()
    }
}

async fn run_gateway_turn_for_seed(
    resolved_path: std::path::PathBuf,
    seed: &OpenAiCompatGatewayTurnSeed,
    observer: Option<crate::mvp::conversation::ConversationTurnObserverHandle>,
) -> Result<crate::mvp::agent_runtime::AgentTurnResult, String> {
    crate::mvp::agent_runtime::AgentRuntime::new()
        .run_turn_with_loaded_config_and_observer_and_error_mode(
            resolved_path,
            seed.run_config.clone(),
            Some(seed.session_id.as_str()),
            &build_openai_compat_turn_request(seed.input.clone()),
            None,
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
    app_state.runtime_dir = runtime_dir;
    app_state.config_path = config_path;
    app_state.config = Some(config);
    build_openai_compat_router(Arc::new(app_state))
}

#[cfg(test)]
fn prepare_openai_compat_test_runtime(config: LoongConfig) -> (std::path::PathBuf, String) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "loong-openai-compat-runtime-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
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
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "loong-openai-compat-turn-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create openai compat turn config dir failed: {error}"))?;

    let config_path = root.join("loong.toml");
    let config_path_text = config_path.display().to_string();
    crate::mvp::config::write(Some(config_path_text.as_str()), config, true)
        .map_err(|error| format!("write openai compat turn config failed: {error}"))?;

    Ok(config_path)
}

#[cfg(test)]
pub(crate) fn build_openai_compat_router(app_state: Arc<GatewayControlAppState>) -> Router {
    Router::new()
        .route("/v1/models", get(handle_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::mvp::config::{
        LoongConfig, ProviderConfig, ProviderKind, ProviderProfileConfig, ProviderWireApi,
    };

    use super::build_openai_compat_test_router_no_backend;

    const OPENAI_COMPAT_TEST_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

    fn run_openai_compat_test_on_large_stack<F, Fut>(thread_name: &str, operation: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let join_handle = std::thread::Builder::new()
            .name(thread_name.to_owned())
            .stack_size(OPENAI_COMPAT_TEST_STACK_SIZE_BYTES)
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build openai compat test runtime");
                runtime.block_on(operation());
            })
            .expect("spawn openai compat large-stack test thread");
        match join_handle.join() {
            Ok(()) => {}
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    fn openai_compat_test_config() -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([
                (
                    "openai-main".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: true,
                        provider: ProviderConfig {
                            kind: ProviderKind::Openai,
                            model: "gpt-5".to_owned(),
                            wire_api: ProviderWireApi::ChatCompletions,
                            ..ProviderConfig::default()
                        },
                    },
                ),
                (
                    "anthropic-main".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: false,
                        provider: ProviderConfig {
                            kind: ProviderKind::Anthropic,
                            model: "claude-sonnet-4-5".to_owned(),
                            ..ProviderConfig::default()
                        },
                    },
                ),
            ]),
            active_provider: Some("openai-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn openai_compat_provider_config(base_url: String) -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([
                (
                    "openai-main".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: true,
                        provider: ProviderConfig {
                            kind: ProviderKind::Openai,
                            model: "gpt-5".to_owned(),
                            base_url: base_url.clone(),
                            api_key: Some(loong_contracts::SecretRef::Inline(
                                "test-key".to_owned(),
                            )),
                            api_key_env: None,
                            oauth_access_token: None,
                            oauth_access_token_env: None,
                            wire_api: ProviderWireApi::ChatCompletions,
                            ..ProviderConfig::default()
                        },
                    },
                ),
                (
                    "anthropic-main".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: false,
                        provider: ProviderConfig {
                            kind: ProviderKind::Anthropic,
                            model: "claude-sonnet-4-5".to_owned(),
                            base_url,
                            api_key: Some(loong_contracts::SecretRef::Inline(
                                "test-key".to_owned(),
                            )),
                            api_key_env: None,
                            oauth_access_token: None,
                            oauth_access_token_env: None,
                            ..ProviderConfig::default()
                        },
                    },
                ),
            ]),
            active_provider: Some("openai-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn next_openai_compat_test_sqlite_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "loong-openai-compat-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn openai_compat_unsupported_stream_config() -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([(
                "bedrock-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Bedrock,
                        model: "anthropic.claude-3-7-sonnet-20250219-v1:0".to_owned(),
                        ..ProviderConfig::default()
                    },
                },
            )]),
            active_provider: Some("bedrock-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn openai_compat_duplicate_model_config(base_url: String) -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([
                (
                    "openai-main".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: true,
                        provider: ProviderConfig {
                            kind: ProviderKind::Openai,
                            model: "gpt-5".to_owned(),
                            base_url: base_url.clone(),
                            api_key: Some(loong_contracts::SecretRef::Inline(
                                "primary-key".to_owned(),
                            )),
                            api_key_env: None,
                            oauth_access_token: None,
                            oauth_access_token_env: None,
                            wire_api: ProviderWireApi::ChatCompletions,
                            ..ProviderConfig::default()
                        },
                    },
                ),
                (
                    "openai-backup".to_owned(),
                    ProviderProfileConfig {
                        default_for_kind: false,
                        provider: ProviderConfig {
                            kind: ProviderKind::Openai,
                            model: "gpt-5".to_owned(),
                            base_url,
                            api_key: Some(loong_contracts::SecretRef::Inline(
                                "backup-key".to_owned(),
                            )),
                            api_key_env: None,
                            oauth_access_token: None,
                            oauth_access_token_env: None,
                            wire_api: ProviderWireApi::ChatCompletions,
                            ..ProviderConfig::default()
                        },
                    },
                ),
            ]),
            active_provider: Some("openai-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn spawn_openai_compat_provider_server(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        spawn_openai_compat_provider_server_with_content_type(status_line, "application/json", body)
    }

    fn spawn_openai_compat_provider_server_with_content_type(
        status_line: &'static str,
        content_type: &'static str,
        body: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider listener");
        let addr = listener.local_addr().expect("local addr");
        let server = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set listener nonblocking");
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut idle_deadline = None;
            let mut requests = Vec::new();
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_openai_compat_provider_request(&mut stream, deadline);
                        requests.push(request);
                        idle_deadline = Some(Instant::now() + Duration::from_secs(2));
                        let response = format!(
                            "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("write response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Some(idle_deadline) = idle_deadline
                            && Instant::now() >= idle_deadline
                        {
                            break;
                        }
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept provider request: {error}"),
                }
            }
            if requests.is_empty() {
                panic!("timed out waiting for provider request");
            }
            requests
        });
        (format!("http://{addr}"), server)
    }

    fn read_openai_compat_provider_request(
        stream: &mut std::net::TcpStream,
        deadline: Instant,
    ) -> String {
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("set read timeout");
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];
        let mut header_end = None;
        let mut expected_total_len = None;
        loop {
            if Instant::now() >= deadline {
                break;
            }
            match stream.read(&mut temp) {
                Ok(0) => break,
                Ok(read) => {
                    buffer.extend_from_slice(&temp[..read]);
                    if header_end.is_none()
                        && let Some(index) =
                            buffer.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let end = index + 4;
                        header_end = Some(end);
                        let headers = String::from_utf8_lossy(&buffer[..end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let lower = line.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length: ")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        expected_total_len = Some(end + content_length);
                    }
                    if let Some(total_len) = expected_total_len
                        && buffer.len() >= total_len
                    {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => panic!("read provider request: {error}"),
            }
        }
        String::from_utf8(buffer).expect("utf8 request")
    }

    #[tokio::test]
    async fn gateway_openai_models_rejects_missing_auth() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_missing_auth() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn gateway_openai_models_lists_configured_provider_profiles() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        if status != axum::http::StatusCode::OK {
            panic!("status={status} body={}", String::from_utf8_lossy(&body));
        }
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let ids = payload["data"]
            .as_array()
            .expect("models array")
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"gpt-5"));
        assert!(ids.contains(&"claude-sonnet-4-5"));
    }

    #[tokio::test]
    async fn gateway_openai_models_exposes_default_configured_model_value() {
        let app =
            build_openai_compat_test_router_no_backend(LoongConfig::default(), "tok".to_owned());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "body={}",
            String::from_utf8_lossy(&body)
        );
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let ids = payload["data"]
            .as_array()
            .expect("models array")
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"auto"), "ids={ids:?}");
    }

    #[test]
    fn gateway_openai_models_disambiguate_duplicate_model_ids_by_profile() {
        run_openai_compat_test_on_large_stack("openai-compat-duplicate-models", || async move {
            gateway_openai_models_disambiguate_duplicate_model_ids_by_profile_impl().await;
        });
    }

    async fn gateway_openai_models_disambiguate_duplicate_model_ids_by_profile_impl() {
        let (base_url, server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_duplicate_model_config(base_url),
            "tok".to_owned(),
        );
        let models_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(models_response.status(), axum::http::StatusCode::OK);
        let models_body = to_bytes(models_response.into_body(), usize::MAX)
            .await
            .expect("body");
        let models_payload: serde_json::Value = serde_json::from_slice(&models_body).expect("json");
        let ids = models_payload["data"]
            .as_array()
            .expect("models array")
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"openai-main:gpt-5"), "ids={ids:?}");
        assert!(ids.contains(&"openai-backup:gpt-5"), "ids={ids:?}");
        assert!(!ids.contains(&"gpt-5"), "ids={ids:?}");

        let completion_body = serde_json::json!({
            "model": "openai-backup:gpt-5",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let completion_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&completion_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        let completion_status = completion_response.status();
        let completion_body = to_bytes(completion_response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(
            completion_status,
            axum::http::StatusCode::OK,
            "body={}",
            String::from_utf8_lossy(&completion_body)
        );
        let requests = server.join().expect("join provider server");
        assert_eq!(requests.len(), 1);
        let normalized_request = requests[0].to_ascii_lowercase();
        assert!(normalized_request.contains("authorization: bearer backup-key"));
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_tools_fields() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{"type": "function", "function": {"name": "echo"}}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_tool_choice_field() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}],
            "tool_choice": "auto"
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["error"]["param"], "tool_choice");
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_unknown_model() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "missing-model",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_messages_not_ending_with_user() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "prior answer"}
            ]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["error"]["param"], "messages");
    }

    #[test]
    fn gateway_openai_chat_completion_returns_non_streaming_response() {
        run_openai_compat_test_on_large_stack("openai-compat-non-stream", || async move {
            gateway_openai_chat_completion_returns_non_streaming_response_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_returns_non_streaming_response_impl() {
        let (base_url, server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
        );
        let mut config = openai_compat_provider_config(base_url);
        config.provider.retry_max_attempts = 1;
        if let Some(profile) = config.providers.get_mut("openai-main") {
            profile.provider.retry_max_attempts = 1;
        }
        let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [
                {"role": "system", "content": "system prompt"},
                {"role": "assistant", "content": "prior answer"},
                {"role": "user", "content": "hello"}
            ]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "body={}",
            String::from_utf8_lossy(&body)
        );
        let requests = server.join().expect("join provider server");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("POST /v1/chat/completions "));
        assert!(
            requests[0].contains("system prompt"),
            "request={}",
            requests[0]
        );
        assert!(
            requests[0].contains("prior answer"),
            "request={}",
            requests[0]
        );
        assert!(requests[0].contains("hello"), "request={}", requests[0]);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");

        assert_eq!(payload["object"], "chat.completion");
        assert_eq!(payload["model"], "gpt-5");
        assert_eq!(payload["choices"][0]["message"]["role"], "assistant");
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            "provider says hi"
        );
    }

    #[test]
    fn gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response() {
        run_openai_compat_test_on_large_stack("openai-compat-usage", || async move {
            gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response_impl()
                .await;
        });
    }

    async fn gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response_impl()
    {
        let (base_url, _server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}"#,
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            payload["usage"],
            json!({
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "total_tokens": 18
            })
        );
    }

    #[test]
    fn gateway_openai_chat_completion_preserves_provider_rate_limit_status() {
        run_openai_compat_test_on_large_stack("openai-compat-rate-limit-status", || async move {
            gateway_openai_chat_completion_preserves_provider_rate_limit_status_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_preserves_provider_rate_limit_status_impl() {
        let (base_url, _server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 429 Too Many Requests",
            r#"{"error":{"message":"rate limit exceeded"}}"#,
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            status,
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "payload={payload}"
        );
        assert!(
            payload["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("rate limit"),
            "payload={payload}"
        );
    }

    #[test]
    fn gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime() {
        run_openai_compat_test_on_large_stack("openai-compat-history", || async move {
            gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime_impl()
                .await;
        });
    }

    async fn gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime_impl() {
        let (base_url, server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
        );
        let sqlite_path = next_openai_compat_test_sqlite_path("history");
        let mut config = openai_compat_provider_config(base_url);
        config.memory.sqlite_path = sqlite_path.display().to_string();
        let memory_config =
            crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
                &config.memory,
            );
        let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
        let body = serde_json::json!({
            "model": "gpt-5",
            "messages": [
                {"role": "system", "content": "system prompt"},
                {"role": "assistant", "content": "prior answer"},
                {"role": "user", "content": "hello"}
            ]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        if status != axum::http::StatusCode::OK {
            panic!("status={status} body={}", String::from_utf8_lossy(&body));
        }
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        let session_id = payload["id"].as_str().expect("response id");
        let turns = crate::mvp::memory::window_direct(session_id, 8, &memory_config)
            .expect("session turns");

        assert!(
            turns
                .iter()
                .any(|turn| turn.role == "system" && turn.content == "system prompt"),
            "turns={turns:?}"
        );
        assert!(
            turns
                .iter()
                .any(|turn| turn.role == "assistant" && turn.content == "prior answer"),
            "turns={turns:?}"
        );
        assert!(
            turns
                .iter()
                .any(|turn| turn.role == "assistant" && turn.content == "provider says hi"),
            "turns={turns:?}"
        );

        let _ = std::fs::remove_file(&sqlite_path);
        server.join().expect("join provider server");
    }

    #[test]
    fn gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime() {
        run_openai_compat_test_on_large_stack("openai-compat-stream-history", || async move {
            gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime_impl()
                .await;
        });
    }

    async fn gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime_impl()
     {
        let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"},\"index\":0,\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"world\"},\"index\":0,\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            ),
        );
        let sqlite_path = next_openai_compat_test_sqlite_path("stream-history");
        let mut config = openai_compat_provider_config(base_url);
        config.memory.sqlite_path = sqlite_path.display().to_string();
        let memory_config =
            crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
                &config.memory,
            );
        let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
        let body = serde_json::json!({
            "model": "gpt-5",
            "stream": true,
            "messages": [
                {"role": "assistant", "content": "prior answer"},
                {"role": "user", "content": "hello"}
            ]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        let payloads = body_text
            .split("\n\n")
            .filter_map(|frame| frame.strip_prefix("data: "))
            .filter(|payload| *payload != "[DONE]")
            .filter_map(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
            .collect::<Vec<_>>();
        let session_id = payloads
            .iter()
            .find_map(|payload| payload["id"].as_str())
            .expect("stream chunk id");
        let turns = crate::mvp::memory::window_direct(session_id, 8, &memory_config)
            .expect("session turns");

        assert!(
            turns
                .iter()
                .any(|turn| turn.role == "user" && turn.content == "hello"),
            "turns={turns:?}"
        );
        assert!(
            turns.iter().any(|turn| {
                turn.role == "assistant" && turn.content.starts_with("hello world")
            }),
            "turns={turns:?}"
        );

        let _ = std::fs::remove_file(&sqlite_path);
    }

    #[test]
    fn gateway_openai_chat_completion_passes_tuning_fields_to_provider() {
        run_openai_compat_test_on_large_stack("openai-compat-tuning", || async move {
            gateway_openai_chat_completion_passes_tuning_fields_to_provider_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_passes_tuning_fields_to_provider_impl() {
        let (base_url, server) = spawn_openai_compat_provider_server(
            "HTTP/1.1 200 OK",
            r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "temperature": 0.2,
            "max_tokens": 77,
            "stop": ["END", "HALT"],
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let requests = server.join().expect("join provider server");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].contains("\"temperature\":0.2"),
            "request={}",
            requests[0]
        );
        assert!(
            requests[0].contains("\"max_tokens\":77")
                || requests[0].contains("\"max_completion_tokens\":77"),
            "request={}",
            requests[0]
        );
        assert!(
            requests[0].contains("\"stop\":[\"END\",\"HALT\"]"),
            "request={}",
            requests[0]
        );
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_rejects_invalid_stop_shape() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_test_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "stop": {"bad": true},
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn gateway_openai_chat_completion_returns_streaming_sse_response() {
        run_openai_compat_test_on_large_stack("openai-compat-stream-anthropic", || async move {
            gateway_openai_chat_completion_returns_streaming_sse_response_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_returns_streaming_sse_response_impl() {
        let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello \"}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "claude-sonnet-4-5",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("text/event-stream"));
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        let contents = collect_stream_contents(body_text.as_str());

        assert!(body_text.contains("chat.completion.chunk"));
        assert_eq!(contents.join(""), "hello world");
        assert!(body_text.contains("data: [DONE]"));
    }

    #[test]
    fn gateway_openai_chat_completion_returns_openai_model_streaming_sse_response() {
        run_openai_compat_test_on_large_stack("openai-compat-stream-openai", || async move {
            gateway_openai_chat_completion_returns_openai_model_streaming_sse_response_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_returns_openai_model_streaming_sse_response_impl() {
        let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"},\"index\":0,\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"world\"},\"index\":0,\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            ),
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "gpt-5",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("text/event-stream"));
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        let contents = collect_stream_contents(body_text.as_str());

        assert_eq!(contents.join(""), "hello world");
        assert!(body_text.contains("data: [DONE]"));
    }

    #[tokio::test]
    async fn gateway_openai_chat_completion_streaming_rejects_truly_unsupported_models() {
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_unsupported_stream_config(),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "anthropic.claude-3-7-sonnet-20250219-v1:0",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn gateway_openai_chat_completion_streaming_failure_emits_error_chunk() {
        run_openai_compat_test_on_large_stack("openai-compat-stream-error", || async move {
            gateway_openai_chat_completion_streaming_failure_emits_error_chunk_impl().await;
        });
    }

    async fn gateway_openai_chat_completion_streaming_failure_emits_error_chunk_impl() {
        let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
            "HTTP/1.1 500 Internal Server Error",
            "application/json",
            r#"{"error":{"message":"stream backend failed"}}"#,
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "claude-sonnet-4-5",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("text/event-stream"));
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");

        assert!(body_text.contains("\"error\""));
        assert!(body_text.contains("data: [DONE]"));
    }

    #[test]
    fn gateway_openai_chat_completion_streaming_uses_provider_events_when_available() {
        run_openai_compat_test_on_large_stack(
            "openai-compat-stream-provider-events",
            || async move {
                gateway_openai_chat_completion_streaming_uses_provider_events_when_available_impl()
                    .await;
            },
        );
    }

    async fn gateway_openai_chat_completion_streaming_uses_provider_events_when_available_impl() {
        let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial \"}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
        );
        let app = build_openai_compat_test_router_no_backend(
            openai_compat_provider_config(base_url),
            "tok".to_owned(),
        );
        let body = serde_json::json!({
            "model": "claude-sonnet-4-5",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/chat/completions")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        let contents = collect_stream_contents(body_text.as_str());

        assert_eq!(contents, vec!["partial ".to_owned(), "hello".to_owned()]);
    }

    fn collect_stream_contents(body_text: &str) -> Vec<String> {
        body_text
            .split("\n\n")
            .filter_map(|frame| frame.strip_prefix("data: "))
            .filter(|payload| *payload != "[DONE]")
            .filter_map(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
            .filter_map(|payload| {
                payload
                    .get("choices")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("delta"))
                    .and_then(|delta| delta.get("content"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect()
    }
}
