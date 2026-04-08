use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinHandle,
};

use crate::mvp::acp::AcpSessionManager;
use crate::mvp::config::LoongClawConfig;
use crate::{
    CliResult, build_channels_cli_json_payload,
    collect_runtime_snapshot_cli_state_from_loaded_config, mvp, supervisor::LoadedSupervisorConfig,
};

use super::api_acp::{handle_acp_dispatch, handle_acp_observability, handle_acp_status};
use super::api_events::handle_events;
use super::api_health::handle_health;
use super::api_turn::handle_turn;
use super::event_bus::GatewayEventBus;
use super::read_models::{
    GatewayChannelInventoryReadModel, GatewayOperatorSummaryReadModel,
    GatewayRuntimeSnapshotReadModel, build_acp_observability_read_model,
    build_acp_session_list_read_model, build_acp_status_read_model,
    build_operator_summary_read_model, build_runtime_snapshot_read_model,
};
use super::state::{
    GatewayControlSurfaceBinding, GatewayStopRequestOutcome, gateway_control_token_path,
    load_gateway_owner_status, request_gateway_stop,
};

const GATEWAY_CONTROL_TOKEN_FILE_MODE: u32 = 0o600;
const GATEWAY_CONTROL_RUNTIME_DIR_MODE: u32 = 0o700;
const GATEWAY_ACP_SESSION_LIST_DEFAULT_LIMIT: usize = 50;
const GATEWAY_ACP_SESSION_LIST_MAX_LIMIT: usize = 200;

type GatewayControlJsonResponse = (StatusCode, Json<Value>);

#[derive(Debug, Default, Deserialize)]
struct GatewayAcpSessionsQuery {
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct GatewayAcpStatusQuery {
    session: Option<String>,
    conversation_id: Option<String>,
    route_session_id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct GatewayControlAppState {
    pub(crate) runtime_dir: PathBuf,
    pub(crate) config_path: String,
    pub(crate) bearer_token: String,
    pub(crate) channel_inventory: Arc<GatewayChannelInventoryReadModel>,
    pub(crate) runtime_snapshot: Arc<GatewayRuntimeSnapshotReadModel>,
    pub(crate) event_bus: Option<GatewayEventBus>,
    pub(crate) acp_manager: Option<Arc<AcpSessionManager>>,
    pub(crate) config: Option<LoongClawConfig>,
}

impl GatewayControlAppState {
    /// Minimal state for tests that don't need ACP.
    pub fn test_minimal(bearer_token: String) -> Self {
        use super::read_models::*;
        use serde_json::json;

        let channel_inventory = GatewayChannelInventoryReadModel {
            config: String::new(),
            schema: GatewayChannelInventorySchema {
                version: 1,
                primary_channel_view: "channel_surfaces",
                catalog_view: "channel_catalog",
                legacy_channel_views: &[],
            },
            channels: vec![],
            catalog_only_channels: vec![],
            channel_catalog: vec![],
            channel_surfaces: vec![],
        };
        let runtime_snapshot = GatewayRuntimeSnapshotReadModel {
            config: String::new(),
            schema: GatewayRuntimeSnapshotSchema {
                version: 1,
                surface: "test",
                purpose: "test",
            },
            provider: json!({}),
            context_engine: json!({}),
            memory_system: json!({}),
            acp: json!({}),
            channels: GatewayRuntimeSnapshotChannelsReadModel {
                enabled_channel_ids: vec![],
                enabled_service_channel_ids: vec![],
                inventory: channel_inventory.clone(),
            },
            tool_runtime: json!({}),
            tools: GatewayRuntimeSnapshotToolsReadModel {
                visible_tool_count: 0,
                visible_tool_names: vec![],
                capability_snapshot_sha256: String::new(),
                capability_snapshot: String::new(),
            },
            runtime_plugins: json!({}),
            external_skills: json!({}),
        };
        Self {
            runtime_dir: PathBuf::from("/tmp/test"),
            config_path: String::new(),
            bearer_token,
            channel_inventory: Arc::new(channel_inventory),
            runtime_snapshot: Arc::new(runtime_snapshot),
            event_bus: None,
            acp_manager: None,
            config: None,
        }
    }
}

struct GatewayControlSurfaceRuntime {
    exit_sender: watch::Sender<Option<CliResult<()>>>,
    shutdown_sender: Mutex<Option<oneshot::Sender<()>>>,
    join_handle: Mutex<Option<JoinHandle<CliResult<()>>>>,
}

#[derive(Clone)]
pub struct GatewayControlSurface {
    binding: GatewayControlSurfaceBinding,
    runtime: Arc<GatewayControlSurfaceRuntime>,
}

impl GatewayControlSurface {
    pub fn binding(&self) -> &GatewayControlSurfaceBinding {
        &self.binding
    }

    pub async fn wait_for_unexpected_exit(&self) -> CliResult<String> {
        let exit_result = self.wait_for_exit_result().await?;
        match exit_result {
            Ok(()) => Err("gateway control surface exited unexpectedly".to_owned()),
            Err(error) => Err(error),
        }
    }

    pub async fn shutdown(&self) -> CliResult<()> {
        let shutdown_sender = {
            let sender_guard = self.runtime.shutdown_sender.lock();
            let mut sender_guard = sender_guard.map_err(|error| {
                format!("gateway control surface shutdown lock poisoned: {error}")
            })?;
            sender_guard.take()
        };
        if let Some(shutdown_sender) = shutdown_sender {
            let _ = shutdown_sender.send(());
        }

        let join_handle = {
            let join_guard = self.runtime.join_handle.lock();
            let mut join_guard = join_guard
                .map_err(|error| format!("gateway control surface join lock poisoned: {error}"))?;
            join_guard.take()
        };
        let Some(join_handle) = join_handle else {
            return Ok(());
        };

        join_handle
            .await
            .map_err(|error| format!("gateway control surface task failed to join: {error}"))?
    }

    async fn wait_for_exit_result(&self) -> CliResult<CliResult<()>> {
        let mut exit_receiver = self.runtime.exit_sender.subscribe();
        let initial_result = exit_receiver.borrow().clone();
        if let Some(initial_result) = initial_result {
            return Ok(initial_result);
        }

        exit_receiver
            .changed()
            .await
            .map_err(|error| format!("gateway control surface exit watch failed: {error}"))?;

        let exit_result = exit_receiver.borrow().clone();
        exit_result
            .ok_or_else(|| "gateway control surface exited without reporting a result".to_owned())
    }
}

pub async fn start_gateway_control_surface(
    runtime_dir: &Path,
    loaded_config: &LoadedSupervisorConfig,
    acp_manager: Option<Arc<AcpSessionManager>>,
) -> CliResult<GatewayControlSurface> {
    let channel_inventory = build_gateway_channel_inventory_read_model(loaded_config)?;
    let runtime_snapshot = build_gateway_runtime_snapshot_read_model(loaded_config)?;
    let bearer_token = new_gateway_control_bearer_token();
    let token_path = gateway_control_token_path(runtime_dir);

    write_gateway_control_token_file(token_path.as_path(), bearer_token.as_str())?;

    let listener_address = gateway_control_listener_address();
    let listener_result = TcpListener::bind(listener_address).await;
    let listener = match listener_result {
        Ok(listener) => listener,
        Err(error) => {
            let bind_error = format!("bind gateway control surface failed: {error}");
            let cleanup_result = remove_gateway_control_token_file(token_path.as_path());
            let final_error = merge_gateway_control_errors(bind_error, cleanup_result.err());
            return Err(final_error);
        }
    };

    let local_address_result = listener.local_addr();
    let local_address = match local_address_result {
        Ok(local_address) => local_address,
        Err(error) => {
            let address_error =
                format!("read gateway control surface local address failed: {error}");
            let cleanup_result = remove_gateway_control_token_file(token_path.as_path());
            let final_error = merge_gateway_control_errors(address_error, cleanup_result.err());
            return Err(final_error);
        }
    };

    let bind_address = local_address.ip().to_string();
    let port = local_address.port();
    let binding = GatewayControlSurfaceBinding {
        bind_address,
        port,
        token_path: token_path.clone(),
    };

    let event_bus = if acp_manager.is_some() {
        Some(GatewayEventBus::new(256))
    } else {
        None
    };

    let app_state = GatewayControlAppState {
        runtime_dir: runtime_dir.to_path_buf(),
        config_path: loaded_config.resolved_path.display().to_string(),
        bearer_token,
        channel_inventory: Arc::new(channel_inventory),
        runtime_snapshot: Arc::new(runtime_snapshot),
        event_bus,
        acp_manager,
        config: Some(loaded_config.config.clone()),
    };
    let app_state = Arc::new(app_state);
    let router = build_gateway_control_router(app_state);

    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let (exit_sender, _) = watch::channel::<Option<CliResult<()>>>(None);
    let exit_sender_for_task = exit_sender.clone();
    let token_path_for_task = token_path;
    let join_handle = tokio::spawn(async move {
        let server = axum::serve(listener, router);
        let server = server.with_graceful_shutdown(async move {
            let _ = shutdown_receiver.await;
        });
        let server_result = server
            .await
            .map_err(|error| format!("gateway control surface server failed: {error}"));
        let cleanup_result = remove_gateway_control_token_file(token_path_for_task.as_path());
        let final_result = combine_gateway_control_task_results(server_result, cleanup_result);
        let _ = exit_sender_for_task.send(Some(final_result.clone()));
        final_result
    });

    let runtime = GatewayControlSurfaceRuntime {
        exit_sender,
        shutdown_sender: Mutex::new(Some(shutdown_sender)),
        join_handle: Mutex::new(Some(join_handle)),
    };
    let runtime = Arc::new(runtime);

    Ok(GatewayControlSurface { binding, runtime })
}

fn build_gateway_control_router(app_state: Arc<GatewayControlAppState>) -> Router {
    Router::new()
        .route("/api/gateway/status", get(handle_gateway_status))
        .route("/api/gateway/channels", get(handle_gateway_channels))
        .route(
            "/api/gateway/runtime-snapshot",
            get(handle_gateway_runtime_snapshot),
        )
        .route(
            "/api/gateway/operator-summary",
            get(handle_gateway_operator_summary),
        )
        .route(
            "/api/gateway/acp/sessions",
            get(handle_gateway_acp_sessions),
        )
        .route("/api/gateway/acp/status", get(handle_gateway_acp_status))
        .route(
            "/api/gateway/acp/observability",
            get(handle_gateway_acp_observability),
        )
        .route("/api/gateway/stop", post(handle_gateway_stop))
        .route("/v1/status", get(handle_gateway_status))
        .route("/v1/channels", get(handle_gateway_channels))
        .route("/v1/runtime/snapshot", get(handle_gateway_runtime_snapshot))
        .route("/v1/acp/status", get(handle_acp_status))
        .route("/v1/acp/observability", get(handle_acp_observability))
        .route("/v1/acp/dispatch", get(handle_acp_dispatch))
        .route("/v1/events", get(handle_events))
        .route("/v1/turn", post(handle_turn))
        .route("/health", get(handle_health))
        .with_state(app_state)
}

async fn handle_gateway_status(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let status = load_gateway_owner_status(app_state.runtime_dir.as_path());
    let Some(status) = status else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "status_unavailable",
            "gateway owner status is unavailable",
        );
    };

    let payload_result = serialize_json_value(&status, "gateway status payload");
    match payload_result {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_channels(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let payload = serialize_json_value(
        app_state.channel_inventory.as_ref(),
        "gateway channels payload",
    );
    match payload {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_runtime_snapshot(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let payload = serialize_json_value(
        app_state.runtime_snapshot.as_ref(),
        "gateway runtime snapshot payload",
    );
    match payload {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_operator_summary(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let status = load_gateway_owner_status(app_state.runtime_dir.as_path());
    let Some(status) = status else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "status_unavailable",
            "gateway owner status is unavailable",
        );
    };

    let summary = build_gateway_operator_summary_read_model(
        &status,
        app_state.channel_inventory.as_ref(),
        app_state.runtime_snapshot.as_ref(),
    );
    let payload = serialize_json_value(&summary, "gateway operator summary payload");
    match payload {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_acp_sessions(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Query(query): Query<GatewayAcpSessionsQuery>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let manager = match gateway_control_acp_manager(app_state.as_ref()) {
        Ok(manager) => manager,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            );
        }
    };

    let sessions_result = manager.list_sessions();
    let mut sessions = match sessions_result {
        Ok(sessions) => sessions,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "acp_sessions_unavailable",
                error.as_str(),
            );
        }
    };

    sort_gateway_acp_sessions(sessions.as_mut_slice());
    let matched_count = sessions.len();
    let limit = gateway_acp_session_list_limit(query.limit);
    sessions.truncate(limit);

    let payload = build_acp_session_list_read_model(
        app_state.config_path.as_str(),
        matched_count,
        sessions.as_slice(),
    );
    let payload = match serialize_json_value(&payload, "gateway ACP sessions payload") {
        Ok(payload) => payload,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialize_failed",
                error.as_str(),
            );
        }
    };

    json_response(StatusCode::OK, payload)
}

async fn handle_gateway_acp_status(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Query(query): Query<GatewayAcpStatusQuery>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let config = match gateway_control_config(app_state.as_ref()) {
        Ok(config) => config,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            );
        }
    };
    let manager = match gateway_control_acp_manager(app_state.as_ref()) {
        Ok(manager) => manager,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            );
        }
    };

    let resolved_session_key = crate::resolve_acp_status_session_key(
        config,
        query.session.as_deref(),
        query.conversation_id.as_deref(),
        query.route_session_id.as_deref(),
    );
    let resolved_session_key = match resolved_session_key {
        Ok(resolved_session_key) => resolved_session_key,
        Err(error) if is_gateway_acp_not_found_error(error.as_str()) => {
            return json_error(StatusCode::NOT_FOUND, "not_found", error.as_str());
        }
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, "invalid_selector", error.as_str());
        }
    };

    let status_result = manager
        .get_status(config, resolved_session_key.as_str())
        .await;
    let status = match status_result {
        Ok(status) => status,
        Err(error) if is_gateway_acp_not_found_error(error.as_str()) => {
            return json_error(StatusCode::NOT_FOUND, "not_found", error.as_str());
        }
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "acp_status_unavailable",
                error.as_str(),
            );
        }
    };

    let payload = build_acp_status_read_model(
        app_state.config_path.as_str(),
        query.session.as_deref(),
        query.conversation_id.as_deref(),
        query.route_session_id.as_deref(),
        resolved_session_key.as_str(),
        &status,
    );
    let payload = match serialize_json_value(&payload, "gateway ACP status payload") {
        Ok(payload) => payload,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialize_failed",
                error.as_str(),
            );
        }
    };

    json_response(StatusCode::OK, payload)
}

async fn handle_gateway_acp_observability(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let config = match gateway_control_config(app_state.as_ref()) {
        Ok(config) => config,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            );
        }
    };
    let manager = match gateway_control_acp_manager(app_state.as_ref()) {
        Ok(manager) => manager,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            );
        }
    };

    let snapshot_result = manager.observability_snapshot(config).await;
    let snapshot = match snapshot_result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "acp_observability_unavailable",
                error.as_str(),
            );
        }
    };

    let payload = build_acp_observability_read_model(app_state.config_path.as_str(), &snapshot);
    let payload = match serialize_json_value(&payload, "gateway ACP observability payload") {
        Ok(payload) => payload,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialize_failed",
                error.as_str(),
            );
        }
    };

    json_response(StatusCode::OK, payload)
}

async fn handle_gateway_stop(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let stop_result = request_gateway_stop(app_state.runtime_dir.as_path());
    let outcome = match stop_result {
        Ok(outcome) => outcome,
        Err(error) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "stop_failed",
                error.as_str(),
            );
        }
    };

    let response_status = gateway_stop_outcome_status(outcome);
    let response_message = gateway_stop_outcome_message(outcome);
    let payload = json!({
        "outcome": gateway_stop_outcome_code(outcome),
        "message": response_message,
    });
    json_response(response_status, payload)
}

fn is_gateway_acp_not_found_error(error: &str) -> bool {
    let is_session_error = error.starts_with("ACP session `");
    let is_conversation_error = error.starts_with("ACP conversation `");
    let is_route_error = error.starts_with("ACP route session `");
    let has_registration_marker = error.contains(" is not registered");
    let is_lookup_error = is_session_error || is_conversation_error || is_route_error;
    is_lookup_error && has_registration_marker
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

pub(crate) fn authorize_request_from_state(
    headers: &HeaderMap,
    app_state: &GatewayControlAppState,
) -> CliResult<()> {
    authorize_request(headers, &app_state.bearer_token)
}

fn authorize_request(headers: &HeaderMap, expected_token: &str) -> CliResult<()> {
    let authorization_header = headers.get(AUTHORIZATION);
    let Some(authorization_header) = authorization_header else {
        return Err("missing Authorization header".to_owned());
    };

    let authorization_text = authorization_header
        .to_str()
        .map_err(|error| format!("invalid Authorization header encoding: {error}"))?;
    let bearer_prefix = "Bearer ";
    let provided_token = authorization_text.strip_prefix(bearer_prefix);
    let Some(provided_token) = provided_token else {
        return Err("Authorization header must use Bearer auth".to_owned());
    };

    if !constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes()) {
        return Err("invalid gateway bearer token".to_owned());
    }

    Ok(())
}

fn build_gateway_channel_inventory_read_model(
    loaded_config: &LoadedSupervisorConfig,
) -> CliResult<GatewayChannelInventoryReadModel> {
    let config_path = loaded_config.resolved_path.display().to_string();
    let inventory = mvp::channel::channel_inventory(&loaded_config.config);
    let read_model = build_channels_cli_json_payload(config_path.as_str(), &inventory);
    Ok(read_model)
}

fn build_gateway_runtime_snapshot_read_model(
    loaded_config: &LoadedSupervisorConfig,
) -> CliResult<GatewayRuntimeSnapshotReadModel> {
    let snapshot = collect_runtime_snapshot_cli_state_from_loaded_config(loaded_config)?;
    let read_model = build_runtime_snapshot_read_model(&snapshot);
    Ok(read_model)
}

fn build_gateway_operator_summary_read_model(
    status: &super::state::GatewayOwnerStatus,
    channel_inventory: &GatewayChannelInventoryReadModel,
    runtime_snapshot: &GatewayRuntimeSnapshotReadModel,
) -> GatewayOperatorSummaryReadModel {
    build_operator_summary_read_model(status, channel_inventory, runtime_snapshot)
}

fn gateway_control_config(app_state: &GatewayControlAppState) -> CliResult<&LoongClawConfig> {
    let config = app_state
        .config
        .as_ref()
        .ok_or_else(|| "gateway ACP config is unavailable".to_owned())?;
    Ok(config)
}

fn gateway_control_acp_manager(
    app_state: &GatewayControlAppState,
) -> CliResult<&AcpSessionManager> {
    let manager = app_state
        .acp_manager
        .as_deref()
        .ok_or_else(|| "gateway ACP session manager is unavailable".to_owned())?;
    Ok(manager)
}

fn gateway_acp_session_list_limit(requested_limit: Option<usize>) -> usize {
    let requested_limit = requested_limit.unwrap_or(GATEWAY_ACP_SESSION_LIST_DEFAULT_LIMIT);
    requested_limit.clamp(1, GATEWAY_ACP_SESSION_LIST_MAX_LIMIT)
}

fn sort_gateway_acp_sessions(sessions: &mut [crate::mvp::acp::AcpSessionMetadata]) {
    sessions.sort_by(|left, right| {
        let activity_order = right.last_activity_ms.cmp(&left.last_activity_ms);
        if activity_order == std::cmp::Ordering::Equal {
            return left.session_key.cmp(&right.session_key);
        }
        activity_order
    });
}

fn serialize_json_value<T: Serialize>(value: &T, context: &str) -> CliResult<Value> {
    serde_json::to_value(value).map_err(|error| format!("serialize {context} failed: {error}"))
}

fn gateway_control_listener_address() -> SocketAddrV4 {
    let bind_address = Ipv4Addr::LOCALHOST;
    let bind_port = 0_u16;
    SocketAddrV4::new(bind_address, bind_port)
}

fn new_gateway_control_bearer_token() -> String {
    let random_bytes = rand::random::<[u8; 32]>();
    URL_SAFE_NO_PAD.encode(random_bytes)
}

fn write_gateway_control_token_file(path: &Path, token: &str) -> CliResult<()> {
    ensure_gateway_control_parent_dir(path)?;
    harden_gateway_control_parent_dir(path)?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(GATEWAY_CONTROL_TOKEN_FILE_MODE);
    }
    let open_result = options.open(path);
    let mut file = open_result.map_err(|error| {
        format!(
            "open gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    file.write_all(token.as_bytes()).map_err(|error| {
        format!(
            "write gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "sync gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    harden_gateway_control_token_file(path)
}

fn ensure_gateway_control_parent_dir(path: &Path) -> CliResult<()> {
    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create gateway control token parent directory failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(unix)]
fn harden_gateway_control_parent_dir(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || !parent.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(parent).map_err(|error| {
        format!(
            "read gateway control runtime directory metadata failed for {}: {error}",
            parent.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(GATEWAY_CONTROL_RUNTIME_DIR_MODE);
    fs::set_permissions(parent, permissions).map_err(|error| {
        format!(
            "set gateway control runtime directory permissions failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_gateway_control_parent_dir(_path: &Path) -> CliResult<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_gateway_control_token_file(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "read gateway control token metadata failed for {}: {error}",
            path.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(GATEWAY_CONTROL_TOKEN_FILE_MODE);
    fs::set_permissions(path, permissions).map_err(|error| {
        format!(
            "set gateway control token permissions failed for {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_gateway_control_token_file(_path: &Path) -> CliResult<()> {
    Ok(())
}

fn remove_gateway_control_token_file(path: &Path) -> CliResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove gateway control token file failed for {}: {error}",
            path.display()
        )),
    }
}

fn combine_gateway_control_task_results(
    server_result: CliResult<()>,
    cleanup_result: CliResult<()>,
) -> CliResult<()> {
    match (server_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(server_error), Ok(())) => Err(server_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(server_error), Err(cleanup_error)) => {
            let final_error = format!("{server_error}; {cleanup_error}");
            Err(final_error)
        }
    }
}

fn merge_gateway_control_errors(primary_error: String, secondary_error: Option<String>) -> String {
    let Some(secondary_error) = secondary_error else {
        return primary_error;
    };

    format!("{primary_error}; {secondary_error}")
}

fn gateway_stop_outcome_status(outcome: GatewayStopRequestOutcome) -> StatusCode {
    match outcome {
        GatewayStopRequestOutcome::Requested => StatusCode::ACCEPTED,
        GatewayStopRequestOutcome::AlreadyRequested => StatusCode::ACCEPTED,
        GatewayStopRequestOutcome::AlreadyStopped => StatusCode::OK,
    }
}

fn gateway_stop_outcome_message(outcome: GatewayStopRequestOutcome) -> &'static str {
    match outcome {
        GatewayStopRequestOutcome::Requested => "gateway stop requested",
        GatewayStopRequestOutcome::AlreadyRequested => "gateway stop already requested",
        GatewayStopRequestOutcome::AlreadyStopped => "gateway is not running",
    }
}

fn gateway_stop_outcome_code(outcome: GatewayStopRequestOutcome) -> &'static str {
    match outcome {
        GatewayStopRequestOutcome::Requested => "requested",
        GatewayStopRequestOutcome::AlreadyRequested => "already_requested",
        GatewayStopRequestOutcome::AlreadyStopped => "already_stopped",
    }
}

fn json_response(status_code: StatusCode, payload: Value) -> GatewayControlJsonResponse {
    (status_code, Json(payload))
}

fn json_error(status_code: StatusCode, code: &str, message: &str) -> GatewayControlJsonResponse {
    let payload = json!({
        "error": {
            "code": code,
            "message": message,
        }
    });
    json_response(status_code, payload)
}

/// Minimal router for health endpoint integration tests.
#[doc(hidden)]
pub fn build_gateway_health_test_router() -> Router {
    Router::new().route("/health", get(handle_health))
}

/// Minimal router for SSE events endpoint integration tests.
#[doc(hidden)]
pub fn build_gateway_events_test_router(
    bearer_token: String,
    event_bus: GatewayEventBus,
) -> Router {
    let mut state = GatewayControlAppState::test_minimal(bearer_token);
    state.event_bus = Some(event_bus);
    let app_state = Arc::new(state);
    Router::new()
        .route("/v1/events", get(handle_events))
        .with_state(app_state)
}

/// Minimal router for ACP gateway endpoint integration tests.
#[doc(hidden)]
pub fn build_gateway_acp_test_router(
    bearer_token: String,
    config: LoongClawConfig,
    acp_manager: Arc<AcpSessionManager>,
) -> Router {
    let mut state = GatewayControlAppState::test_minimal(bearer_token);
    state.acp_manager = Some(acp_manager);
    state.config = Some(config);
    let app_state = Arc::new(state);
    Router::new()
        .route("/v1/acp/status", get(handle_acp_status))
        .route("/v1/acp/observability", get(handle_acp_observability))
        .route("/v1/acp/dispatch", get(handle_acp_dispatch))
        .with_state(app_state)
}
