use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::control::{GatewayControlAppState, authorize_request_from_state};
use super::openai_tooling::{
    GatewayToolSettings, apply_gateway_tool_settings, clear_gateway_session_tool_policy,
    resolve_gateway_tool_settings,
};
use super::response_store::GatewayResponseStore;
use crate::mvp::config::{LoongConfig, ProviderProfileConfig};

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesRequest {
    model: String,
    input: Value,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    previous_response_id: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    tools: Option<Value>,
    #[serde(default)]
    tool_choice: Option<Value>,
}

#[derive(Debug, Clone)]
struct ResponsesInputMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone)]
struct ConfiguredResponsesModelBinding {
    request_model_id: String,
    profile_id: String,
    provider: crate::mvp::config::ProviderConfig,
}

#[derive(Debug, Clone)]
struct OpenAiResponsesGatewayTurnSeed {
    response_id: String,
    session_id: String,
    model: String,
    created_at: u64,
    previous_response_id: Option<String>,
    run_config: LoongConfig,
    input: String,
    requested_tool_ids: Option<Vec<String>>,
    disable_tools: bool,
}

struct ResponsesStreamObserver {
    sender: mpsc::UnboundedSender<Result<Event, Infallible>>,
    emitted_text: AtomicBool,
}

impl ResponsesStreamObserver {
    fn new(sender: mpsc::UnboundedSender<Result<Event, Infallible>>) -> Self {
        Self {
            sender,
            emitted_text: AtomicBool::new(false),
        }
    }

    fn emitted_text(&self) -> bool {
        self.emitted_text.load(Ordering::Relaxed)
    }

    fn push_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.emitted_text.store(true, Ordering::Relaxed);
        let payload = json!({
            "type": "response.output_text.delta",
            "delta": text,
        });
        let _ = self.sender.send(Ok(Event::default()
            .event("response.output_text.delta")
            .data(payload.to_string())));
    }
}

impl crate::mvp::conversation::ConversationTurnObserver for ResponsesStreamObserver {
    fn on_streaming_token(&self, event: crate::mvp::acp::StreamingTokenEvent) {
        if event.event_type != "text_delta" {
            return;
        }
        let Some(text) = event.delta.text.as_deref() else {
            return;
        };
        self.push_delta(text);
    }
}

static OPENAI_RESPONSES_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn handle_responses(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ResponsesRequest>,
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
    let Some(response_store) = app_state.response_store.clone() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "gateway response store not available"}),
        );
    };

    let tool_settings = match resolve_gateway_tool_settings(
        config,
        request.tools.as_ref(),
        request.tool_choice.as_ref(),
    ) {
        Ok(tool_settings) => tool_settings,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": error.message, "param": error.param}}),
            );
        }
    };

    let seed = match build_responses_turn_seed(
        app_state.as_ref(),
        config,
        &response_store,
        &request,
        tool_settings,
    ) {
        Ok(seed) => seed,
        Err(ResponsesSeedError::BadRequest(error, param)) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": {"message": error, "param": param}}),
            );
        }
        Err(ResponsesSeedError::NotFound(error)) => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": {"message": error}}));
        }
        Err(ResponsesSeedError::Internal(error)) => {
            return json_response(
                gateway_runtime_error_status(error.as_str()),
                json!({"error": {"message": error}}),
            );
        }
    };

    if request.stream {
        return stream_response_turn(app_state.as_ref(), response_store, seed).await;
    }

    complete_response_turn(app_state.as_ref(), &response_store, seed)
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

pub(crate) async fn handle_get_response(
    headers: HeaderMap,
    Path(response_id): Path<String>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": error}));
    }
    let Some(response_store) = app_state.response_store.clone() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "gateway response store not available"}),
        );
    };
    match response_store.load_response(response_id.as_str()) {
        Ok(Some(record)) => json_response(StatusCode::OK, record.payload),
        Ok(None) => json_response(
            StatusCode::NOT_FOUND,
            json!({"error": {"message": format!("unknown response `{response_id}`")}}),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": {"message": error}}),
        ),
    }
}

pub(crate) async fn handle_delete_response(
    headers: HeaderMap,
    Path(response_id): Path<String>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    if let Err(error) = authorize_request_from_state(&headers, &app_state) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error": error}));
    }
    let Some(response_store) = app_state.response_store.clone() else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "gateway response store not available"}),
        );
    };
    match response_store.delete_response(response_id.as_str()) {
        Ok(true) => json_response(
            StatusCode::OK,
            json!({
                "id": response_id,
                "object": "response.deleted",
                "deleted": true
            }),
        ),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            json!({"error": {"message": format!("unknown response `{response_id}`")}}),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": {"message": error}}),
        ),
    }
}

async fn complete_response_turn(
    app_state: &GatewayControlAppState,
    response_store: &GatewayResponseStore,
    seed: OpenAiResponsesGatewayTurnSeed,
) -> Result<Value, String> {
    let result = run_gateway_response_turn(
        PathBuf::from(app_state.config_path.clone()),
        seed.run_config.clone(),
        seed.session_id.as_str(),
        seed.input.clone(),
        seed.requested_tool_ids.clone(),
        seed.disable_tools,
        None,
    )
    .await?;
    let payload = build_response_payload(&seed, result.output_text.as_str(), result.usage);
    response_store.save_response(
        seed.response_id.as_str(),
        seed.session_id.as_str(),
        &payload,
    )?;
    Ok(payload)
}

async fn stream_response_turn(
    app_state: &GatewayControlAppState,
    response_store: GatewayResponseStore,
    seed: OpenAiResponsesGatewayTurnSeed,
) -> Response {
    let (sender, receiver) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let observer = Arc::new(ResponsesStreamObserver::new(sender.clone()));
    let observer_handle: crate::mvp::conversation::ConversationTurnObserverHandle =
        observer.clone();
    let resolved_path = PathBuf::from(app_state.config_path.clone());
    let streaming_supported =
        crate::mvp::provider::supports_turn_streaming_events(&seed.run_config);

    tokio::spawn(async move {
        let result = run_gateway_response_turn(
            resolved_path,
            seed.run_config.clone(),
            seed.session_id.as_str(),
            seed.input.clone(),
            seed.requested_tool_ids.clone(),
            seed.disable_tools,
            streaming_supported.then_some(observer_handle),
        )
        .await;
        match result {
            Ok(result) => {
                if !observer.emitted_text() && !result.output_text.is_empty() {
                    observer.push_delta(result.output_text.as_str());
                }
                let payload =
                    build_response_payload(&seed, result.output_text.as_str(), result.usage);
                let _ = response_store.save_response(
                    seed.response_id.as_str(),
                    seed.session_id.as_str(),
                    &payload,
                );
                let completed_event = Event::default()
                    .event("response.completed")
                    .data(json!({"type": "response.completed", "response": payload}).to_string());
                let _ = sender.send(Ok(completed_event));
            }
            Err(error) => {
                let failed_event = Event::default().event("response.failed").data(
                    json!({
                        "type": "response.failed",
                        "error": {"message": error}
                    })
                    .to_string(),
                );
                let _ = sender.send(Ok(failed_event));
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

fn build_response_payload(
    seed: &OpenAiResponsesGatewayTurnSeed,
    output_text: &str,
    usage: Option<Value>,
) -> Value {
    let output = json!([{
        "id": format!("msg_{}", seed.response_id),
        "type": "message",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": output_text,
            "annotations": []
        }]
    }]);

    json!({
        "id": seed.response_id,
        "object": "response",
        "created_at": seed.created_at,
        "status": "completed",
        "model": seed.model,
        "previous_response_id": seed.previous_response_id,
        "output": output,
        "usage": usage.unwrap_or(Value::Null)
    })
}

fn next_openai_responses_request_id(model: &str) -> String {
    let counter = OPENAI_RESPONSES_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("resp-openai-gateway-{model}-{counter}")
}

fn current_unix_timestamp_secs() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock before unix epoch: {error}"))
}

#[derive(Debug)]
enum ResponsesSeedError {
    BadRequest(String, &'static str),
    NotFound(String),
    Internal(String),
}

fn build_responses_turn_seed(
    _app_state: &GatewayControlAppState,
    config: &LoongConfig,
    response_store: &GatewayResponseStore,
    request: &ResponsesRequest,
    tool_settings: GatewayToolSettings,
) -> Result<OpenAiResponsesGatewayTurnSeed, ResponsesSeedError> {
    let input_messages = parse_response_input(&request.input)
        .map_err(|error| ResponsesSeedError::BadRequest(error, "input"))?;
    if input_messages.is_empty() {
        return Err(ResponsesSeedError::BadRequest(
            "input must not be empty".to_owned(),
            "input",
        ));
    }
    let binding = configured_provider_for_response(config, request)
        .map_err(|error| ResponsesSeedError::BadRequest(error, "model"))?;
    let response_id = next_openai_responses_request_id(request.model.as_str());
    let created_at = current_unix_timestamp_secs().map_err(ResponsesSeedError::Internal)?;
    let mut run_config = config.clone();
    run_config.provider = binding.provider;
    run_config.active_provider = Some(binding.profile_id);

    let previous_response_id = request
        .previous_response_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let (session_id, input) = if let Some(previous_response_id) = previous_response_id.as_deref() {
        let Some((last_message, history)) = input_messages.split_last() else {
            return Err(ResponsesSeedError::BadRequest(
                "input must not be empty".to_owned(),
                "input",
            ));
        };
        if last_message.role != "user" {
            return Err(ResponsesSeedError::BadRequest(
                "continued responses must end with a `user` message".to_owned(),
                "input",
            ));
        }
        let session_id = response_store
            .resolve_session_id(previous_response_id)
            .map_err(ResponsesSeedError::Internal)?
            .ok_or_else(|| {
                ResponsesSeedError::NotFound(format!(
                    "unknown previous_response_id `{previous_response_id}`"
                ))
            })?;
        if !history.is_empty() {
            append_response_history_messages(session_id.as_str(), history, config)
                .map_err(ResponsesSeedError::Internal)?;
        }
        if tool_settings.disable_tools {
            clear_gateway_session_tool_policy(config, session_id.as_str())
                .map_err(ResponsesSeedError::Internal)?;
        } else if tool_settings.requested_tool_ids.is_some() {
            apply_gateway_tool_settings(config, session_id.as_str(), &tool_settings)
                .map_err(ResponsesSeedError::Internal)?;
        }
        (session_id, last_message.content.clone())
    } else {
        let Some((last_message, history)) = input_messages.split_last() else {
            return Err(ResponsesSeedError::BadRequest(
                "input must not be empty".to_owned(),
                "input",
            ));
        };
        if last_message.role != "user" {
            return Err(ResponsesSeedError::BadRequest(
                "input must end with a `user` message on this gateway surface".to_owned(),
                "input",
            ));
        }
        let session_id = response_id.clone();
        if tool_settings.disable_tools {
            clear_gateway_session_tool_policy(config, session_id.as_str())
                .map_err(ResponsesSeedError::Internal)?;
        } else if tool_settings.requested_tool_ids.is_some() {
            apply_gateway_tool_settings(config, session_id.as_str(), &tool_settings)
                .map_err(ResponsesSeedError::Internal)?;
        }
        if !history.is_empty() {
            seed_initial_response_history(session_id.as_str(), history, &run_config)
                .map_err(ResponsesSeedError::Internal)?;
        }
        (session_id, last_message.content.clone())
    };

    Ok(OpenAiResponsesGatewayTurnSeed {
        response_id,
        session_id,
        model: request.model.clone(),
        created_at,
        previous_response_id,
        run_config,
        input,
        requested_tool_ids: tool_settings.requested_tool_ids,
        disable_tools: tool_settings.disable_tools,
    })
}

fn seed_initial_response_history(
    session_id: &str,
    history: &[ResponsesInputMessage],
    run_config: &LoongConfig,
) -> Result<(), String> {
    let history_turns = history
        .iter()
        .enumerate()
        .map(|(index, message)| crate::mvp::memory::WindowTurn {
            role: message.role.clone(),
            content: message.content.clone(),
            ts: Some(index as i64),
        })
        .collect::<Vec<_>>();
    let memory_config = crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
        &run_config.memory,
    );
    crate::mvp::memory::execute_memory_core_with_config(
        crate::mvp::memory::build_replace_turns_request(session_id, &history_turns),
        &memory_config,
    )
    .map(|_| ())
    .map_err(|error| format!("seed gateway responses session failed: {error}"))
}

fn append_response_history_messages(
    session_id: &str,
    history: &[ResponsesInputMessage],
    config: &LoongConfig,
) -> Result<(), String> {
    let store_config =
        crate::mvp::session::store::session_store_config_from_memory_config(&config.memory);
    for message in history {
        crate::mvp::session::store::append_session_turn_direct(
            session_id,
            message.role.as_str(),
            message.content.as_str(),
            &store_config,
        )
        .map_err(|error| format!("append gateway responses history failed: {error}"))?;
    }
    Ok(())
}

fn parse_response_input(input: &Value) -> Result<Vec<ResponsesInputMessage>, String> {
    if let Some(text) = input.as_str() {
        return Ok(vec![ResponsesInputMessage {
            role: "user".to_owned(),
            content: text.to_owned(),
        }]);
    }
    if let Some(items) = input.as_array() {
        let mut messages = Vec::new();
        for item in items {
            messages.extend(parse_response_input_item(item)?);
        }
        return Ok(messages);
    }
    if input.is_object() {
        return parse_response_input_item(input);
    }
    Err("input must be a string, object, or array".to_owned())
}

fn parse_response_input_item(item: &Value) -> Result<Vec<ResponsesInputMessage>, String> {
    let item_type = item.get("type").and_then(Value::as_str);
    match item_type {
        Some("message") => Ok(vec![parse_response_message_item(item)?]),
        Some("input_text") | Some("text") => {
            let Some(text) = item.get("text").and_then(Value::as_str) else {
                return Err("input_text item is missing `text`".to_owned());
            };
            Ok(vec![ResponsesInputMessage {
                role: "user".to_owned(),
                content: text.to_owned(),
            }])
        }
        Some(other) => Err(format!("unsupported input item type `{other}`")),
        None => {
            if item.get("role").is_some() {
                Ok(vec![parse_response_message_item(item)?])
            } else if let Some(text) = item.get("text").and_then(Value::as_str) {
                Ok(vec![ResponsesInputMessage {
                    role: "user".to_owned(),
                    content: text.to_owned(),
                }])
            } else {
                Err("unsupported input item shape".to_owned())
            }
        }
    }
}

fn parse_response_message_item(item: &Value) -> Result<ResponsesInputMessage, String> {
    let Some(role) = item.get("role").and_then(Value::as_str) else {
        return Err("message input item is missing `role`".to_owned());
    };
    if !matches!(role, "system" | "user" | "assistant") {
        return Err(format!("unsupported input message role `{role}`"));
    }
    let Some(content) = item.get("content") else {
        return Err("message input item is missing `content`".to_owned());
    };
    let content = render_response_message_content(content)?;
    Ok(ResponsesInputMessage {
        role: role.to_owned(),
        content,
    })
}

fn render_response_message_content(content: &Value) -> Result<String, String> {
    if let Some(text) = content.as_str() {
        return Ok(text.to_owned());
    }
    let Some(parts) = content.as_array() else {
        return Err("unsupported input message content shape".to_owned());
    };
    let mut text = String::new();
    for part in parts {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("text");
        if !matches!(part_type, "input_text" | "output_text" | "text") {
            return Err(format!("unsupported input content part type `{part_type}`"));
        }
        let Some(part_text) = part.get("text").and_then(Value::as_str) else {
            return Err("input content part is missing `text`".to_owned());
        };
        text.push_str(part_text);
    }
    Ok(text)
}

fn configured_provider_for_response(
    config: &LoongConfig,
    request: &ResponsesRequest,
) -> Result<ConfiguredResponsesModelBinding, String> {
    let mut binding = resolve_response_model_binding(config, request.model.as_str())
        .ok_or_else(|| format!("unknown model `{}`", request.model))?;
    if let Some(temperature) = request.temperature {
        binding.provider.temperature = temperature;
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        binding.provider.max_tokens = Some(max_output_tokens);
    }
    Ok(binding)
}

fn resolve_response_model_binding(
    config: &LoongConfig,
    model: &str,
) -> Option<ConfiguredResponsesModelBinding> {
    configured_response_model_bindings(config)
        .into_iter()
        .find(|binding| binding.request_model_id == model)
}

fn configured_response_model_bindings(
    config: &LoongConfig,
) -> Vec<ConfiguredResponsesModelBinding> {
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
            bindings.push(ConfiguredResponsesModelBinding {
                request_model_id,
                profile_id: profile_id.clone(),
                provider: bound_provider,
            });
        }
    }
    bindings
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

fn configured_provider_model_ids(provider: &crate::mvp::config::ProviderConfig) -> Vec<String> {
    if let Some(explicit_model) = provider.explicit_model() {
        return vec![explicit_model];
    }
    if !provider.configured_auto_model_candidates().is_empty() {
        return provider.configured_auto_model_candidates();
    }
    vec![provider.configured_model_value()]
}

fn build_responses_turn_request(input: String) -> crate::mvp::agent_runtime::AgentTurnRequest {
    crate::mvp::agent_runtime::AgentTurnRequest {
        message: input,
        turn_mode: crate::mvp::agent_runtime::AgentTurnMode::Oneshot,
        ..Default::default()
    }
}

async fn run_gateway_response_turn(
    resolved_path: PathBuf,
    run_config: LoongConfig,
    session_id: &str,
    input: String,
    requested_tool_ids: Option<Vec<String>>,
    disable_tools: bool,
    observer: Option<crate::mvp::conversation::ConversationTurnObserverHandle>,
) -> Result<crate::mvp::agent_runtime::AgentTurnResult, String> {
    crate::mvp::agent_runtime::AgentRuntime::new()
        .run_turn_with_loaded_config_and_observer_and_error_mode(
            resolved_path,
            run_config,
            Some(session_id),
            &build_responses_turn_request(input)
                .with_requested_tool_ids(requested_tool_ids)
                .with_disable_tools(disable_tools),
            None,
            observer,
            crate::mvp::conversation::ProviderErrorMode::Propagate,
        )
        .await
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

#[doc(hidden)]
pub fn build_openai_responses_test_router_no_backend(
    config: LoongConfig,
    bearer_token: String,
    sqlite_path: PathBuf,
) -> Router {
    let mut app_state = GatewayControlAppState::test_minimal(bearer_token);
    app_state.config = Some(config);
    app_state.response_store =
        Some(GatewayResponseStore::new(sqlite_path).expect("initialize gateway response store"));
    Router::new()
        .route("/v1/responses", post(handle_responses))
        .route(
            "/v1/responses/{response_id}",
            get(handle_get_response).delete(handle_delete_response),
        )
        .with_state(Arc::new(app_state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    use crate::mvp::config::{
        LoongConfig, ProviderConfig, ProviderKind, ProviderProfileConfig, ProviderWireApi,
    };

    fn next_openai_responses_test_sqlite_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "loong-openai-responses-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn openai_responses_chat_provider_config(base_url: String) -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([(
                "openai-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        base_url,
                        api_key: Some(loong_contracts::SecretRef::Inline("test-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        wire_api: ProviderWireApi::ChatCompletions,
                        ..ProviderConfig::default()
                    },
                },
            )]),
            active_provider: Some("openai-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn openai_responses_api_provider_config(base_url: String) -> LoongConfig {
        LoongConfig {
            providers: BTreeMap::from([(
                "openai-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        base_url,
                        api_key: Some(loong_contracts::SecretRef::Inline("test-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        wire_api: ProviderWireApi::Responses,
                        ..ProviderConfig::default()
                    },
                },
            )]),
            active_provider: Some("openai-main".to_owned()),
            ..LoongConfig::default()
        }
    }

    fn spawn_openai_responses_provider_server(
        responses: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider listener");
        let addr = listener.local_addr().expect("local addr");
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut requests = Vec::new();
            for (status_line, content_type, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept provider request");
                let request = read_openai_responses_provider_request(&mut stream, deadline);
                requests.push(request);
                let response = format!(
                    "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write provider response");
            }
            requests
        });
        (format!("http://{addr}"), server)
    }

    fn read_openai_responses_provider_request(
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
        String::from_utf8(buffer).expect("provider request utf8")
    }

    fn collect_response_delta_events(body_text: &str) -> Vec<String> {
        body_text
            .split("\n\n")
            .filter(|frame| frame.contains("event: response.output_text.delta"))
            .filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
            .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
            .filter_map(|payload| {
                payload
                    .get("delta")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect()
    }

    #[tokio::test]
    async fn gateway_openai_responses_rejects_missing_auth() {
        let sqlite_path = next_openai_responses_test_sqlite_path("missing-auth");
        let app = build_openai_responses_test_router_no_backend(
            LoongConfig::default(),
            "tok".to_owned(),
            sqlite_path,
        );
        let body = json!({"model": "gpt-5", "input": "hello"});
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn gateway_openai_responses_round_trip_get_and_delete() {
        let (base_url, _server) = spawn_openai_responses_provider_server(vec![(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"pong"}]}]}"#,
        )]);
        let sqlite_path = next_openai_responses_test_sqlite_path("round-trip");
        let app = build_openai_responses_test_router_no_backend(
            openai_responses_api_provider_config(base_url),
            "tok".to_owned(),
            sqlite_path.clone(),
        );
        let create_body = json!({"model": "gpt-5", "input": "ping"});
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&create_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("create response");

        assert_eq!(create_response.status(), StatusCode::OK);
        let create_body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .expect("create body");
        let create_payload: Value = serde_json::from_slice(&create_body).expect("create json");
        let response_id = create_payload["id"].as_str().expect("response id");
        assert_eq!(create_payload["object"], "response");
        assert_eq!(create_payload["output"][0]["content"][0]["text"], "pong");

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/responses/{response_id}"))
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("get response");
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_body = to_bytes(get_response.into_body(), usize::MAX)
            .await
            .expect("get body");
        let get_payload: Value = serde_json::from_slice(&get_body).expect("get json");
        assert_eq!(get_payload["id"], response_id);
        assert_eq!(get_payload["output"][0]["content"][0]["text"], "pong");

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/responses/{response_id}"))
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("delete response");
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_body = to_bytes(delete_response.into_body(), usize::MAX)
            .await
            .expect("delete body");
        let delete_payload: Value = serde_json::from_slice(&delete_body).expect("delete json");
        assert_eq!(delete_payload["deleted"], true);

        let missing_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/responses/{response_id}"))
                    .header("authorization", "Bearer tok")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("missing response");
        assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn gateway_openai_responses_previous_response_reuses_session_history() {
        let (base_url, server) = spawn_openai_responses_provider_server(vec![
            (
                "HTTP/1.1 200 OK",
                "application/json",
                r#"{"choices":[{"message":{"role":"assistant","content":"first answer"}}]}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                "application/json",
                r#"{"choices":[{"message":{"role":"assistant","content":"second answer"}}]}"#,
            ),
        ]);
        let sqlite_path = next_openai_responses_test_sqlite_path("continuity");
        let app = build_openai_responses_test_router_no_backend(
            openai_responses_chat_provider_config(base_url),
            "tok".to_owned(),
            sqlite_path.clone(),
        );

        let first_body = json!({"model": "gpt-5", "input": "hello"});
        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&first_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("first response");
        assert_eq!(first_response.status(), StatusCode::OK);
        let first_body = to_bytes(first_response.into_body(), usize::MAX)
            .await
            .expect("first body");
        let first_payload: Value = serde_json::from_slice(&first_body).expect("first json");
        let first_response_id = first_payload["id"].as_str().expect("first response id");

        let second_body = json!({
            "model": "gpt-5",
            "previous_response_id": first_response_id,
            "input": "followup"
        });
        let second_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&second_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("second response");
        assert_eq!(second_response.status(), StatusCode::OK);

        let requests = server.join().expect("join provider server");
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains("\"content\":\"hello\""),
            "second request should include prior user turn: {:#?}",
            requests
        );
        assert!(
            requests[1].contains("\"content\":\"first answer\""),
            "second request should include prior assistant turn: {:#?}",
            requests
        );
        assert!(
            requests[1].contains("\"content\":\"followup\""),
            "second request should include current user turn: {:#?}",
            requests
        );

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn gateway_openai_responses_previous_response_accepts_multiple_new_input_messages() {
        let (base_url, server) = spawn_openai_responses_provider_server(vec![
            (
                "HTTP/1.1 200 OK",
                "application/json",
                r#"{"choices":[{"message":{"role":"assistant","content":"first answer"}}]}"#,
            ),
            (
                "HTTP/1.1 200 OK",
                "application/json",
                r#"{"choices":[{"message":{"role":"assistant","content":"second answer"}}]}"#,
            ),
        ]);
        let sqlite_path = next_openai_responses_test_sqlite_path("continued-multi-input");
        let app = build_openai_responses_test_router_no_backend(
            openai_responses_chat_provider_config(base_url),
            "tok".to_owned(),
            sqlite_path.clone(),
        );

        let first_body = json!({"model": "gpt-5", "input": "hello"});
        let first_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&first_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("first response");
        assert_eq!(first_response.status(), StatusCode::OK);
        let first_body = to_bytes(first_response.into_body(), usize::MAX)
            .await
            .expect("first body");
        let first_payload: Value = serde_json::from_slice(&first_body).expect("first json");
        let first_response_id = first_payload["id"].as_str().expect("first response id");

        let second_body = json!({
            "model": "gpt-5",
            "previous_response_id": first_response_id,
            "input": [
                {"type": "message", "role": "assistant", "content": "carry this forward"},
                {"type": "message", "role": "user", "content": "followup"}
            ]
        });
        let second_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&second_body).expect("encode body"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("second response");
        assert_eq!(second_response.status(), StatusCode::OK);

        let requests = server.join().expect("join provider server");
        assert_eq!(requests.len(), 2);
        assert!(
            requests[1].contains("\"content\":\"carry this forward\""),
            "continued response should append new assistant context before the new user turn: {:#?}",
            requests
        );
        assert!(
            requests[1].contains("\"content\":\"followup\""),
            "continued response should carry the last user message into the new turn: {:#?}",
            requests
        );

        let _ = std::fs::remove_file(sqlite_path);
    }

    #[tokio::test]
    async fn gateway_openai_responses_streaming_emits_delta_and_completed_events() {
        let (base_url, _server) = spawn_openai_responses_provider_server(vec![(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"streamed fallback"}]}]}"#,
        )]);
        let sqlite_path = next_openai_responses_test_sqlite_path("streaming");
        let app = build_openai_responses_test_router_no_backend(
            openai_responses_api_provider_config(base_url),
            "tok".to_owned(),
            sqlite_path.clone(),
        );
        let body = json!({
            "model": "gpt-5",
            "stream": true,
            "input": "hello"
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("authorization", "Bearer tok")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(content_type.starts_with("text/event-stream"));
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
        let deltas = collect_response_delta_events(body_text.as_str());
        assert_eq!(deltas, vec!["streamed fallback".to_owned()]);
        assert!(body_text.contains("event: response.completed"));
        assert!(body_text.contains("\"text\":\"streamed fallback\""));

        let _ = std::fs::remove_file(sqlite_path);
    }
}
