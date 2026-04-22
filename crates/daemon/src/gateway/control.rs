use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{
        IntoResponse, Response,
        sse::{KeepAlive, Sse},
    },
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use loong_protocol::{
    ControlPlaneChallengeResponse, ControlPlaneConnectErrorCode, ControlPlaneConnectErrorResponse,
    ControlPlaneConnectRequest, ControlPlanePairingListResponse, ControlPlanePairingRequestSummary,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse,
    ControlPlanePairingStatus, ControlPlanePrincipal, ControlPlaneRole, ControlPlaneScope,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinHandle,
};

use crate::mvp::acp::AcpSessionManager;
use crate::mvp::config::LoongConfig;
use crate::{
    CliResult, build_channels_cli_json_payload,
    collect_runtime_snapshot_cli_state_from_loaded_config, mvp, supervisor::LoadedSupervisorConfig,
};

use super::api_acp::{handle_acp_dispatch, handle_acp_observability, handle_acp_status};
use super::api_events::{
    GatewayEventsQuery, bounded_gateway_event_limit, gateway_event_stream, handle_events,
};
use super::api_health::handle_health;
use super::api_turn::handle_turn;
use super::event_bus::GatewayEventBus;
use super::openai_compat::{handle_chat_completions, handle_models};
use super::read_models::{
    GatewayChannelInventoryReadModel, GatewayOperatorPairingSummaryReadModel,
    GatewayOperatorSummaryReadModel, GatewayPairingSessionLeaseReadModel,
    GatewayRuntimeSnapshotReadModel, build_acp_observability_read_model,
    build_acp_session_list_read_model, build_acp_status_read_model,
    build_gateway_pairing_complete_read_model, build_gateway_pairing_events_read_model,
    build_gateway_pairing_session_read_model, build_gateway_pairing_start_read_model,
    build_operator_summary_read_model, build_runtime_snapshot_read_model,
};
use super::state::{
    GatewayControlSurfaceBinding, GatewayPairingRuntimeState, GatewayPortSource,
    GatewayStopRequestOutcome, gateway_control_token_path, load_gateway_owner_status,
    load_gateway_pairing_runtime_state, request_gateway_stop, write_gateway_pairing_runtime_state,
};

#[cfg(unix)]
const GATEWAY_CONTROL_TOKEN_FILE_MODE: u32 = 0o600;
#[cfg(unix)]
const GATEWAY_CONTROL_RUNTIME_DIR_MODE: u32 = 0o700;
const GATEWAY_ACP_SESSION_LIST_DEFAULT_LIMIT: usize = 50;
const GATEWAY_ACP_SESSION_LIST_MAX_LIMIT: usize = 200;
const GATEWAY_CONTROL_PORT_ENV: &str = "LOONGCLAW_GATEWAY_PORT";
const GATEWAY_PAIRING_CHALLENGE_MAX_FUTURE_SKEW_MS: u64 = 30_000;

type GatewayControlJsonResponse = (StatusCode, Json<Value>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GatewayPortResolution {
    port: u16,
    source: GatewayPortSource,
}

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

#[derive(Debug, Default, Deserialize)]
struct GatewayPairingListQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct GatewayPairingEventsQuery {
    after_seq: Option<u64>,
    limit: Option<usize>,
    ack_seq: Option<u64>,
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
    pub(crate) challenge_registry: Arc<mvp::control_plane::ControlPlaneChallengeRegistry>,
    pub(crate) connection_registry: Arc<mvp::control_plane::ControlPlaneConnectionRegistry>,
    pub(crate) config: Option<LoongConfig>,
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
            summary: GatewayChannelInventorySummaryReadModel {
                total_surface_count: 0,
                runtime_backed_surface_count: 0,
                config_backed_surface_count: 0,
                plugin_backed_surface_count: 0,
                catalog_only_surface_count: 0,
            },
            channels: vec![],
            catalog_only_channels: vec![],
            channel_catalog: vec![],
            channel_surfaces: vec![],
            channel_access_policies: vec![],
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
                enabled_runtime_backed_channel_ids: vec![],
                enabled_service_channel_ids: vec![],
                enabled_plugin_backed_channel_ids: vec![],
                enabled_outbound_only_channel_ids: vec![],
                inventory: channel_inventory.clone(),
            },
            tool_runtime: json!({}),
            tools: GatewayRuntimeSnapshotToolsReadModel {
                visible_tool_count: 0,
                visible_tool_names: vec![],
                visible_direct_tool_names: vec![],
                hidden_tool_count: 0,
                hidden_tool_tags: vec![],
                hidden_tool_surfaces: vec![],
                capability_snapshot_sha256: String::new(),
                capability_snapshot: String::new(),
                tool_calling: super::read_models::GatewayToolCallingReadModel {
                    availability: "inactive".to_owned(),
                    structured_tool_schema_enabled: false,
                    effective_tool_schema_mode: "enabled_with_downgrade".to_owned(),
                    active_model: String::new(),
                    reason: "no runtime-visible tools are enabled".to_owned(),
                },
                web_access: super::read_models::GatewayWebAccessReadModel {
                    ordinary_network_access_enabled: false,
                    query_search_enabled: false,
                    query_search_default_provider: "duckduckgo".to_owned(),
                    query_search_credential_ready: true,
                    separation_note: crate::RUNTIME_WEB_ACCESS_SEPARATION_NOTE.to_owned(),
                },
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
            challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
            connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
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
    port_override: Option<u16>,
) -> CliResult<GatewayControlSurface> {
    let channel_inventory = build_gateway_channel_inventory_read_model(loaded_config)?;
    let runtime_snapshot = build_gateway_runtime_snapshot_read_model(loaded_config)?;
    let bearer_token = new_gateway_control_bearer_token();
    let token_path = gateway_control_token_path(runtime_dir);
    let persisted_pairing_runtime = load_gateway_pairing_runtime_state(runtime_dir);

    write_gateway_control_token_file(token_path.as_path(), bearer_token.as_str())?;

    let port_resolution =
        resolve_gateway_control_listener_port(&loaded_config.config, port_override)?;
    let listener_address = gateway_control_listener_address_from_port_resolution(port_resolution);
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
        port_source: port_resolution.source,
        token_path: token_path.clone(),
    };

    let connection_registry = Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new());
    if let Some(persisted_pairing_runtime) = persisted_pairing_runtime.as_ref() {
        connection_registry
            .restore_leases(&persisted_pairing_runtime.sessions)
            .map_err(|error| format!("restore gateway pairing sessions failed: {error}"))?;
    }
    let event_bus = match (persisted_pairing_runtime.as_ref(), acp_manager.is_some()) {
        (Some(persisted_pairing_runtime), _) => Some(GatewayEventBus::from_snapshot(
            256,
            persisted_pairing_runtime.event_bus.clone(),
        )),
        (None, true) => Some(GatewayEventBus::new(256)),
        (None, false) => None,
    };

    let app_state = GatewayControlAppState {
        runtime_dir: runtime_dir.to_path_buf(),
        config_path: loaded_config.resolved_path.display().to_string(),
        bearer_token,
        channel_inventory: Arc::new(channel_inventory),
        runtime_snapshot: Arc::new(runtime_snapshot),
        event_bus,
        acp_manager,
        challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
        connection_registry,
        config: Some(loaded_config.config.clone()),
    };
    let app_state = Arc::new(app_state);
    attach_gateway_pairing_runtime_persist_hook(app_state.clone());
    let app_state_for_task = app_state.clone();
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
        let persist_result = persist_gateway_pairing_runtime_state(app_state_for_task.as_ref());
        let server_result = combine_gateway_control_task_results(server_result, persist_result);
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
        .route(
            "/api/gateway/pairing/requests",
            get(handle_gateway_pairing_requests),
        )
        .route(
            "/api/gateway/pairing/start",
            post(handle_gateway_pairing_start),
        )
        .route(
            "/api/gateway/pairing/resolve",
            post(handle_gateway_pairing_resolve),
        )
        .route(
            "/api/gateway/pairing/complete",
            post(handle_gateway_pairing_complete),
        )
        .route(
            "/api/gateway/pairing/session",
            get(handle_gateway_pairing_session),
        )
        .route(
            "/api/gateway/pairing/events",
            get(handle_gateway_pairing_events),
        )
        .route(
            "/api/gateway/pairing/stream",
            get(handle_gateway_pairing_stream),
        )
        .route("/api/gateway/stop", post(handle_gateway_stop))
        .route("/v1/status", get(handle_gateway_status))
        .route("/v1/channels", get(handle_gateway_channels))
        .route("/v1/runtime/snapshot", get(handle_gateway_runtime_snapshot))
        .route("/v1/acp/status", get(handle_acp_status))
        .route("/v1/acp/observability", get(handle_acp_observability))
        .route("/v1/acp/dispatch", get(handle_acp_dispatch))
        .route("/v1/pairing/start", post(handle_gateway_pairing_start))
        .route("/v1/pairing/requests", get(handle_gateway_pairing_requests))
        .route("/v1/pairing/resolve", post(handle_gateway_pairing_resolve))
        .route(
            "/v1/pairing/complete",
            post(handle_gateway_pairing_complete),
        )
        .route("/v1/pairing/session", get(handle_gateway_pairing_session))
        .route("/v1/pairing/events", get(handle_gateway_pairing_events))
        .route("/v1/pairing/stream", get(handle_gateway_pairing_stream))
        .route("/v1/events", get(handle_events))
        .route("/v1/turn", post(handle_turn))
        .route("/v1/models", get(handle_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
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
        app_state.as_ref(),
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

async fn handle_gateway_pairing_requests(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Query(query): Query<GatewayPairingListQuery>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let pairing_registry = match gateway_pairing_registry(app_state.as_ref()) {
        Ok(pairing_registry) => pairing_registry,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "pairing_unavailable",
                error.as_str(),
            );
        }
    };

    let status = match query.status.as_deref() {
        Some(raw) => match parse_gateway_pairing_status(raw) {
            Ok(status) => Some(status),
            Err(error) => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_pairing_status",
                    error.as_str(),
                );
            }
        },
        None => None,
    };
    let limit = query.limit.unwrap_or(50);
    let requests = pairing_registry.list_requests(status, limit);
    let payload = ControlPlanePairingListResponse {
        matched_count: requests.len(),
        returned_count: requests.len(),
        requests: requests
            .into_iter()
            .map(map_gateway_pairing_request)
            .collect::<Vec<_>>(),
    };
    let payload = match serialize_json_value(&payload, "gateway pairing requests payload") {
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

async fn handle_gateway_pairing_start(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let challenge = app_state.challenge_registry.issue();
    let challenge = ControlPlaneChallengeResponse {
        nonce: challenge.nonce,
        issued_at_ms: challenge.issued_at_ms,
        expires_at_ms: challenge.expires_at_ms,
    };
    let payload = build_gateway_pairing_start_read_model(challenge);
    let payload = match serialize_json_value(&payload, "gateway pairing start payload") {
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

async fn handle_gateway_pairing_resolve(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ControlPlanePairingResolveRequest>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let pairing_registry = match gateway_pairing_registry(app_state.as_ref()) {
        Ok(pairing_registry) => pairing_registry,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "pairing_unavailable",
                error.as_str(),
            );
        }
    };

    match pairing_registry.resolve_request(request.pairing_request_id.as_str(), request.approve) {
        Ok(Some(record)) => {
            let payload = ControlPlanePairingResolveResponse {
                request: map_gateway_pairing_request(record.clone()),
                device_token: record.device_token,
            };
            let payload = match serialize_json_value(&payload, "gateway pairing resolve payload") {
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
        Ok(None) => json_error(
            StatusCode::NOT_FOUND,
            "pairing_not_found",
            format!(
                "pairing request `{}` not found",
                request.pairing_request_id.trim()
            )
            .as_str(),
        ),
        Err(error) => json_error(
            StatusCode::BAD_REQUEST,
            "pairing_resolve_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_pairing_complete(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
    Json(request): Json<ControlPlaneConnectRequest>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    if request.max_protocol < loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
        || request.min_protocol > loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
    {
        return json_connect_error(
            StatusCode::BAD_REQUEST,
            ControlPlaneConnectErrorCode::ProtocolMismatch,
            format!(
                "protocol mismatch: expected protocol {}",
                loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION
            ),
        );
    }

    let device = match request.device.as_ref() {
        Some(device) => device,
        None => {
            return json_connect_error(
                StatusCode::BAD_REQUEST,
                ControlPlaneConnectErrorCode::ChallengeRequired,
                "gateway pairing complete requires device identity",
            );
        }
    };

    if let Err(response) = verify_gateway_pairing_device_challenge(app_state.as_ref(), &request) {
        return response;
    }

    let pairing_registry = match gateway_pairing_registry(app_state.as_ref()) {
        Ok(pairing_registry) => pairing_registry,
        Err(error) => {
            return json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "pairing_unavailable",
                error.as_str(),
            );
        }
    };

    let requested_scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let device_token = request
        .auth
        .as_ref()
        .and_then(|auth| auth.device_token.as_deref());

    match pairing_registry.evaluate_connect(
        device.device_id.as_str(),
        request.client.id.as_str(),
        device.public_key.as_str(),
        request.role.as_str(),
        &requested_scopes,
        device_token,
    ) {
        Ok(mvp::control_plane::ControlPlanePairingConnectDecision::Authorized) => {
            let requested_scopes = request.scopes.iter().copied().collect::<Vec<_>>();
            let lease = issue_gateway_pairing_session_lease(app_state.as_ref(), &request);
            let _ = persist_gateway_pairing_runtime_state(app_state.as_ref());
            let payload = build_gateway_pairing_complete_read_model(
                device.device_id.as_str(),
                request.client.id.as_str(),
                request.role,
                requested_scopes,
                lease,
            );
            let payload = match serialize_json_value(&payload, "gateway pairing complete payload") {
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
        Ok(mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
            request: pairing_request,
            ..
        }) => json_connect_error_with_request(
            StatusCode::FORBIDDEN,
            ControlPlaneConnectErrorCode::PairingRequired,
            format!(
                "device `{}` requires operator pairing approval before connect can complete",
                pairing_request.device_id
            ),
            Some(pairing_request.pairing_request_id.clone()),
        ),
        Ok(mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenRequired) => {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::DeviceTokenRequired,
                format!(
                    "device `{}` is paired but must present auth.device_token on connect",
                    device.device_id
                ),
            )
        }
        Ok(mvp::control_plane::ControlPlanePairingConnectDecision::DeviceTokenInvalid) => {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::DeviceTokenInvalid,
                format!(
                    "device `{}` presented an invalid auth.device_token",
                    device.device_id
                ),
            )
        }
        Err(error) => json_error(
            StatusCode::BAD_REQUEST,
            "pairing_complete_failed",
            error.as_str(),
        ),
    }
}

async fn handle_gateway_pairing_session(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    let lease = match authorize_gateway_pairing_session_scope_request(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(lease) => lease,
        Err(response) => return response,
    };

    let principal = gateway_pairing_protocol_principal(&lease);
    let replay_window = app_state
        .event_bus
        .as_ref()
        .map(GatewayEventBus::replay_window)
        .unwrap_or(super::event_bus::GatewayEventReplayWindow {
            oldest_retained_seq: None,
            latest_seq: None,
        });
    let payload = build_gateway_pairing_session_read_model(
        GatewayPairingSessionLeaseReadModel {
            connection_token: lease.token,
            connection_token_expires_at_ms: lease.expires_at_ms,
            principal,
            last_acknowledged_seq: lease.acknowledged_seq,
        },
        replay_window,
    );
    let payload = match serialize_json_value(&payload, "gateway pairing session payload") {
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

async fn handle_gateway_pairing_events(
    headers: HeaderMap,
    Query(query): Query<GatewayPairingEventsQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    let session_token = match extract_gateway_pairing_session_token(&headers) {
        Some(token) => token,
        None => {
            return json_error(
                StatusCode::UNAUTHORIZED,
                "missing_session_token",
                "missing gateway pairing session token",
            );
        }
    };

    let lease = match authorize_gateway_pairing_session_scope_request(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(lease) => lease,
        Err(response) => return response,
    };

    let Some(event_bus) = app_state.event_bus.as_ref() else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "event_stream_unavailable",
            "gateway event streaming is not available",
        );
    };

    let after_seq = query.after_seq.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 256);
    let lease = if let Some(ack_seq) = query.ack_seq {
        match app_state
            .connection_registry
            .acknowledge_seq(session_token.as_str(), ack_seq)
        {
            Ok(Some(updated)) => updated,
            Ok(None) => {
                return json_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_session_token",
                    "invalid or expired gateway pairing session token",
                );
            }
            Err(error) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_registry_failed",
                    error.as_str(),
                );
            }
        }
    } else {
        lease
    };
    if query.ack_seq.is_some() {
        let _ = persist_gateway_pairing_runtime_state(app_state.as_ref());
    }
    let replay_window = event_bus.replay_window();
    if gateway_pairing_after_seq_is_stale(after_seq, replay_window) {
        let message = match (replay_window.oldest_retained_seq, replay_window.latest_seq) {
            (Some(oldest), Some(latest)) => format!(
                "requested after_seq={} is older than retained replay window {}..{}",
                after_seq, oldest, latest
            ),
            _ => format!("requested after_seq={after_seq} is outside the retained replay window"),
        };
        return json_stale_cursor_error(message.as_str(), lease.acknowledged_seq, replay_window);
    }
    let events = event_bus.recent_events_after(after_seq, limit);
    let payload = build_gateway_pairing_events_read_model(
        after_seq,
        lease.acknowledged_seq,
        replay_window,
        events,
    );
    let payload = match serialize_json_value(&payload, "gateway pairing events payload") {
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

async fn handle_gateway_pairing_stream(
    headers: HeaderMap,
    Query(query): Query<GatewayEventsQuery>,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> Response {
    let lease = match authorize_gateway_pairing_session_scope_request(
        &headers,
        app_state.as_ref(),
        ControlPlaneScope::OperatorRead,
    ) {
        Ok(lease) => lease,
        Err(response) => return response.into_response(),
    };

    let Some(event_bus) = app_state.event_bus.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "gateway event streaming is not available"})),
        )
            .into_response();
    };

    let after_seq = query.after_seq.unwrap_or(0);
    let replay_window = event_bus.replay_window();
    if gateway_pairing_after_seq_is_stale(after_seq, replay_window) {
        let message = match (replay_window.oldest_retained_seq, replay_window.latest_seq) {
            (Some(oldest), Some(latest)) => format!(
                "requested after_seq={} is older than retained replay window {}..{}",
                after_seq, oldest, latest
            ),
            _ => format!("requested after_seq={after_seq} is outside the retained replay window"),
        };
        return json_stale_cursor_error(message.as_str(), lease.acknowledged_seq, replay_window)
            .into_response();
    }

    let limit = bounded_gateway_event_limit(query.limit);
    let event_stream = gateway_event_stream(event_bus.clone(), query.after_seq, limit);
    Sse::new(event_stream)
        .keep_alive(KeepAlive::default())
        .into_response()
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

pub(crate) fn is_gateway_acp_not_found_error(error: &str) -> bool {
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
    app_state: &GatewayControlAppState,
) -> GatewayOperatorSummaryReadModel {
    let pairing = build_gateway_pairing_summary_read_model(app_state);
    build_operator_summary_read_model(status, channel_inventory, runtime_snapshot, pairing)
}

fn build_gateway_pairing_summary_read_model(
    app_state: &GatewayControlAppState,
) -> GatewayOperatorPairingSummaryReadModel {
    match gateway_pairing_registry(app_state) {
        Ok(pairing_registry) => GatewayOperatorPairingSummaryReadModel {
            pending_request_count: pairing_registry.pending_request_count(),
            approved_device_count: pairing_registry.approved_device_count(),
            last_activity_ms: pairing_registry.last_activity_ms(),
        },
        Err(_) => GatewayOperatorPairingSummaryReadModel {
            pending_request_count: 0,
            approved_device_count: 0,
            last_activity_ms: None,
        },
    }
}

fn attach_gateway_pairing_runtime_persist_hook(app_state: Arc<GatewayControlAppState>) {
    let Some(event_bus) = app_state.event_bus.as_ref() else {
        return;
    };
    let weak_app_state: Weak<GatewayControlAppState> = Arc::downgrade(&app_state);
    event_bus.set_publish_hook(Arc::new(move || {
        if let Some(app_state) = weak_app_state.upgrade() {
            let _ = persist_gateway_pairing_runtime_state(app_state.as_ref());
        }
    }));
}

fn persist_gateway_pairing_runtime_state(app_state: &GatewayControlAppState) -> CliResult<()> {
    let sessions = app_state.connection_registry.snapshot_leases();
    let max_acknowledged_seq = sessions
        .iter()
        .filter_map(|lease| lease.acknowledged_seq)
        .max()
        .unwrap_or(0);
    let event_bus_snapshot = app_state
        .event_bus
        .as_ref()
        .map(GatewayEventBus::snapshot)
        .unwrap_or(super::event_bus::GatewayEventBusSnapshot {
            next_seq: 0,
            recent_events: Vec::new(),
        });
    let event_bus = super::event_bus::GatewayEventBusSnapshot {
        next_seq: event_bus_snapshot.next_seq.max(max_acknowledged_seq),
        recent_events: event_bus_snapshot.recent_events,
    };
    let state = GatewayPairingRuntimeState {
        sessions,
        event_bus,
    };
    write_gateway_pairing_runtime_state(app_state.runtime_dir.as_path(), &state)
}

fn gateway_control_config(app_state: &GatewayControlAppState) -> CliResult<&LoongConfig> {
    let config = app_state
        .config
        .as_ref()
        .ok_or_else(|| "gateway config is unavailable".to_owned())?;
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

fn default_gateway_control_listener_address(config: &LoongConfig) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.gateway.port)
}

fn gateway_control_listener_address_from_port_resolution(
    resolution: GatewayPortResolution,
) -> SocketAddrV4 {
    let bind_address = Ipv4Addr::LOCALHOST;
    let bind_port = resolution.port;
    SocketAddrV4::new(bind_address, bind_port)
}

fn resolve_gateway_control_listener_port(
    config: &LoongConfig,
    port_override: Option<u16>,
) -> CliResult<GatewayPortResolution> {
    if let Some(port_override) = port_override {
        return Ok(GatewayPortResolution {
            port: port_override,
            source: if port_override == 0 {
                GatewayPortSource::EphemeralCli
            } else {
                GatewayPortSource::Cli
            },
        });
    }

    if let Some(port) = resolve_gateway_control_listener_port_from_env()? {
        return Ok(GatewayPortResolution {
            port,
            source: GatewayPortSource::Env,
        });
    }

    if config.gateway.port != mvp::config::GatewayConfig::default().port {
        return Ok(GatewayPortResolution {
            port: config.gateway.port,
            source: GatewayPortSource::Config,
        });
    }

    let default_port = default_gateway_control_listener_address(config).port();
    Ok(GatewayPortResolution {
        port: default_port,
        source: GatewayPortSource::Default,
    })
}

fn resolve_gateway_control_listener_port_from_env() -> CliResult<Option<u16>> {
    let Some(raw_value) = std::env::var_os(GATEWAY_CONTROL_PORT_ENV) else {
        return Ok(None);
    };
    let raw_value = raw_value.to_string_lossy();
    let trimmed_value = raw_value.trim();
    if trimmed_value.is_empty() {
        return Ok(None);
    }
    let port = trimmed_value.parse::<u16>().map_err(|error| {
        format!("parse {GATEWAY_CONTROL_PORT_ENV}=`{trimmed_value}` failed: {error}")
    })?;
    Ok(Some(port))
}

fn gateway_pairing_registry(
    app_state: &GatewayControlAppState,
) -> CliResult<mvp::control_plane::ControlPlanePairingRegistry> {
    let config = gateway_control_config(app_state)?;
    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config =
            crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                &config.memory,
            );
        mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(memory_config)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = config;
        Err("gateway pairing requires sqlite memory support".to_owned())
    }
}

fn issue_gateway_pairing_session_lease(
    app_state: &GatewayControlAppState,
    request: &ControlPlaneConnectRequest,
) -> GatewayPairingSessionLeaseReadModel {
    let connection_id = format!(
        "gwp-{:016x}",
        gateway_current_time_ms().saturating_add(rand::random::<u32>() as u64)
    );
    let principal = mvp::control_plane::ControlPlaneConnectionPrincipal {
        connection_id,
        client_id: request.client.id.clone(),
        role: request.role.as_str().to_owned(),
        scopes: request
            .scopes
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect(),
        device_id: request
            .device
            .as_ref()
            .map(|device| device.device_id.clone()),
    };
    let lease = app_state.connection_registry.issue(principal);
    let principal = gateway_pairing_protocol_principal(&lease);
    GatewayPairingSessionLeaseReadModel {
        connection_token: lease.token,
        connection_token_expires_at_ms: lease.expires_at_ms,
        principal,
        last_acknowledged_seq: lease.acknowledged_seq,
    }
}

fn gateway_pairing_protocol_principal(
    lease: &mvp::control_plane::ControlPlaneConnectionLease,
) -> ControlPlanePrincipal {
    let role = match lease.principal.role.as_str() {
        "operator" => ControlPlaneRole::Operator,
        _ => ControlPlaneRole::Node,
    };
    let scopes = lease
        .principal
        .scopes
        .iter()
        .filter_map(|scope| ControlPlaneScope::parse(scope.as_str()))
        .collect();
    ControlPlanePrincipal {
        connection_id: lease.principal.connection_id.clone(),
        client_id: lease.principal.client_id.clone(),
        role,
        scopes,
        device_id: lease.principal.device_id.clone(),
    }
}

fn authorize_gateway_pairing_session_request(
    headers: &HeaderMap,
    app_state: &GatewayControlAppState,
) -> Result<mvp::control_plane::ControlPlaneConnectionLease, GatewayControlJsonResponse> {
    let Some(token) = extract_gateway_pairing_session_token(headers) else {
        return Err(json_error(
            StatusCode::UNAUTHORIZED,
            "missing_session_token",
            "missing gateway pairing session token",
        ));
    };
    let lease = app_state
        .connection_registry
        .resolve(token.as_str())
        .map_err(|error| {
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session_registry_failed",
                error.as_str(),
            )
        })?;
    let Some(lease) = lease else {
        return Err(json_error(
            StatusCode::UNAUTHORIZED,
            "invalid_session_token",
            "invalid or expired gateway pairing session token",
        ));
    };
    Ok(lease)
}

fn authorize_gateway_pairing_session_scope_request(
    headers: &HeaderMap,
    app_state: &GatewayControlAppState,
    required_scope: ControlPlaneScope,
) -> Result<mvp::control_plane::ControlPlaneConnectionLease, GatewayControlJsonResponse> {
    let lease = authorize_gateway_pairing_session_request(headers, app_state)?;
    let has_required_scope = lease.principal.scopes.iter().any(|scope| {
        scope == required_scope.as_str() || scope == ControlPlaneScope::OperatorAdmin.as_str()
    });
    if !has_required_scope {
        return Err(json_error(
            StatusCode::FORBIDDEN,
            "insufficient_scope",
            "gateway pairing session token does not grant the required scope",
        ));
    }
    Ok(lease)
}

fn extract_gateway_pairing_session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-loongclaw-pairing-session-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn gateway_pairing_after_seq_is_stale(
    after_seq: u64,
    replay_window: super::event_bus::GatewayEventReplayWindow,
) -> bool {
    let Some(oldest_retained_seq) = replay_window.oldest_retained_seq else {
        return false;
    };
    after_seq < oldest_retained_seq.saturating_sub(1)
}

fn verify_gateway_pairing_device_challenge(
    app_state: &GatewayControlAppState,
    request: &ControlPlaneConnectRequest,
) -> Result<(), GatewayControlJsonResponse> {
    let device = request.device.as_ref().ok_or_else(|| {
        json_connect_error(
            StatusCode::BAD_REQUEST,
            ControlPlaneConnectErrorCode::ChallengeRequired,
            "gateway pairing complete requires device identity",
        )
    })?;

    let challenge = app_state
        .challenge_registry
        .consume(device.nonce.as_str())
        .map_err(|error| {
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "challenge_registry_failed",
                error.as_str(),
            )
        })?
        .ok_or_else(|| {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::ChallengeExpired,
                format!(
                    "unknown or expired control-plane challenge `{}`",
                    device.nonce
                ),
            )
        })?;

    let now_ms = gateway_current_time_ms();
    if device.signed_at_ms < challenge.issued_at_ms
        || device.signed_at_ms
            > challenge
                .expires_at_ms
                .saturating_add(GATEWAY_PAIRING_CHALLENGE_MAX_FUTURE_SKEW_MS)
        || device.signed_at_ms > now_ms.saturating_add(GATEWAY_PAIRING_CHALLENGE_MAX_FUTURE_SKEW_MS)
    {
        return Err(json_connect_error(
            StatusCode::UNAUTHORIZED,
            ControlPlaneConnectErrorCode::ChallengeExpired,
            format!(
                "control-plane device signature timestamp is outside the challenge window for `{}`",
                device.device_id
            ),
        ));
    }

    let public_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.public_key.as_bytes())
        .map_err(|error| {
            json_error(
                StatusCode::BAD_REQUEST,
                "invalid_device_public_key_encoding",
                format!("invalid control-plane device public_key encoding: {error}").as_str(),
            )
        })?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.signature.as_bytes())
        .map_err(|error| {
            json_error(
                StatusCode::BAD_REQUEST,
                "invalid_device_signature_encoding",
                format!("invalid control-plane device signature encoding: {error}").as_str(),
            )
        })?;
    let public_key_array: [u8; 32] = public_key_bytes.try_into().map_err(|_length_error| {
        json_error(
            StatusCode::BAD_REQUEST,
            "invalid_device_public_key_length",
            "control-plane device public_key must decode to 32 bytes",
        )
    })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_array).map_err(|error| {
        json_error(
            StatusCode::BAD_REQUEST,
            "invalid_device_public_key",
            format!("invalid control-plane device public_key: {error}").as_str(),
        )
    })?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|error| {
        json_error(
            StatusCode::BAD_REQUEST,
            "invalid_device_signature",
            format!("invalid control-plane device signature bytes: {error}").as_str(),
        )
    })?;
    let message = gateway_pairing_device_signature_message(request, device);
    verifying_key
        .verify(&message, &signature)
        .map_err(|error| {
            json_connect_error(
                StatusCode::UNAUTHORIZED,
                ControlPlaneConnectErrorCode::DeviceSignatureInvalid,
                format!("control-plane device signature verification failed: {error}"),
            )
        })?;

    Ok(())
}

fn gateway_pairing_device_signature_message(
    request: &ControlPlaneConnectRequest,
    device: &loong_protocol::ControlPlaneDeviceIdentity,
) -> Vec<u8> {
    let scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "loong-control-plane-connect-v1\nnonce={}\ndevice_id={}\nclient_id={}\nrole={}\nscopes={}\nsigned_at_ms={}",
        device.nonce,
        device.device_id,
        request.client.id,
        request.role.as_str(),
        scopes,
        device.signed_at_ms
    )
    .into_bytes()
}

fn map_gateway_pairing_status(
    status: mvp::control_plane::ControlPlanePairingStatus,
) -> ControlPlanePairingStatus {
    match status {
        mvp::control_plane::ControlPlanePairingStatus::Pending => {
            ControlPlanePairingStatus::Pending
        }
        mvp::control_plane::ControlPlanePairingStatus::Approved => {
            ControlPlanePairingStatus::Approved
        }
        mvp::control_plane::ControlPlanePairingStatus::Rejected => {
            ControlPlanePairingStatus::Rejected
        }
    }
}

fn map_gateway_pairing_request(
    request: mvp::control_plane::ControlPlanePairingRequestRecord,
) -> ControlPlanePairingRequestSummary {
    ControlPlanePairingRequestSummary {
        pairing_request_id: request.pairing_request_id,
        device_id: request.device_id,
        client_id: request.client_id,
        public_key: request.public_key,
        role: match request.role.as_str() {
            "operator" => ControlPlaneRole::Operator,
            _ => ControlPlaneRole::Node,
        },
        requested_scopes: request
            .requested_scopes
            .into_iter()
            .filter_map(|scope| ControlPlaneScope::parse(scope.as_str()))
            .collect::<std::collections::BTreeSet<_>>(),
        status: map_gateway_pairing_status(request.status),
        requested_at_ms: request.requested_at_ms,
        resolved_at_ms: request.resolved_at_ms,
    }
}

fn parse_gateway_pairing_status(
    raw: &str,
) -> Result<mvp::control_plane::ControlPlanePairingStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(mvp::control_plane::ControlPlanePairingStatus::Pending),
        "approved" => Ok(mvp::control_plane::ControlPlanePairingStatus::Approved),
        "rejected" => Ok(mvp::control_plane::ControlPlanePairingStatus::Rejected),
        _ => Err(format!("unknown pairing status `{raw}`")),
    }
}

fn json_connect_error(
    status_code: StatusCode,
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> GatewayControlJsonResponse {
    json_connect_error_with_request(status_code, code, error, None)
}

fn json_connect_error_with_request(
    status_code: StatusCode,
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
    pairing_request_id: Option<String>,
) -> GatewayControlJsonResponse {
    let payload = ControlPlaneConnectErrorResponse {
        code,
        error: error.into(),
        pairing_request_id,
    };
    let payload = serde_json::to_value(&payload)
        .unwrap_or_else(|_| json!({"error": "failed to serialize connect error"}));
    json_response(status_code, payload)
}

fn json_stale_cursor_error(
    message: &str,
    last_acknowledged_seq: Option<u64>,
    replay_window: super::event_bus::GatewayEventReplayWindow,
) -> GatewayControlJsonResponse {
    let earliest_resumable_after_seq = replay_window
        .oldest_retained_seq
        .map(|seq| seq.saturating_sub(1))
        .unwrap_or(0);
    let payload = json!({
        "error": {
            "code": "stale_cursor",
            "message": message,
            "last_acknowledged_seq": last_acknowledged_seq,
            "earliest_resumable_after_seq": earliest_resumable_after_seq,
            "replay_window": {
                "oldest_retained_seq": replay_window.oldest_retained_seq,
                "latest_seq": replay_window.latest_seq,
            }
        }
    });
    json_response(StatusCode::CONFLICT, payload)
}

fn gateway_current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
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
    config: LoongConfig,
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

/// Minimal router for gateway pairing endpoint integration tests.
#[doc(hidden)]
pub fn build_gateway_pairing_test_router(bearer_token: String, config: LoongConfig) -> Router {
    let event_bus = GatewayEventBus::new(64);
    build_gateway_pairing_test_router_with_event_bus(bearer_token, config, event_bus)
}

#[doc(hidden)]
pub fn build_gateway_pairing_test_router_with_event_bus(
    bearer_token: String,
    config: LoongConfig,
    event_bus: GatewayEventBus,
) -> Router {
    let mut state = GatewayControlAppState::test_minimal(bearer_token);
    state.event_bus = Some(event_bus);
    state.config = Some(config);
    let app_state = Arc::new(state);
    Router::new()
        .route("/v1/pairing/start", post(handle_gateway_pairing_start))
        .route("/v1/pairing/requests", get(handle_gateway_pairing_requests))
        .route("/v1/pairing/resolve", post(handle_gateway_pairing_resolve))
        .route(
            "/v1/pairing/complete",
            post(handle_gateway_pairing_complete),
        )
        .route("/v1/pairing/session", get(handle_gateway_pairing_session))
        .route("/v1/pairing/events", get(handle_gateway_pairing_events))
        .route("/v1/pairing/stream", get(handle_gateway_pairing_stream))
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::{
        GATEWAY_CONTROL_PORT_ENV, gateway_control_listener_address_from_port_resolution,
        resolve_gateway_control_listener_port,
    };
    use crate::gateway::state::GatewayPortSource;
    use crate::mvp::config::LoongConfig;
    use crate::test_support::ScopedEnv;

    #[test]
    fn gateway_control_listener_port_defaults_to_26306() {
        let mut env = ScopedEnv::new();
        env.remove(GATEWAY_CONTROL_PORT_ENV);

        let config = LoongConfig::default();
        let resolution = resolve_gateway_control_listener_port(&config, None).expect("resolution");
        let listener_address = gateway_control_listener_address_from_port_resolution(resolution);

        assert_eq!(*listener_address.ip(), std::net::Ipv4Addr::LOCALHOST);
        assert_eq!(listener_address.port(), 26_306);
        assert_eq!(resolution.source, GatewayPortSource::Default);
    }

    #[test]
    fn gateway_control_listener_port_uses_env_override() {
        let mut env = ScopedEnv::new();
        env.set(GATEWAY_CONTROL_PORT_ENV, "26316");

        let config = LoongConfig::default();
        let resolution = resolve_gateway_control_listener_port(&config, None).expect("resolution");

        assert_eq!(resolution.port, 26_316);
        assert_eq!(resolution.source, GatewayPortSource::Env);
    }

    #[test]
    fn gateway_control_listener_port_uses_config_value_without_override() {
        let mut env = ScopedEnv::new();
        env.remove(GATEWAY_CONTROL_PORT_ENV);

        let mut config = LoongConfig::default();
        config.gateway.port = 26_346;
        let resolution = resolve_gateway_control_listener_port(&config, None).expect("resolution");

        assert_eq!(resolution.port, 26_346);
        assert_eq!(resolution.source, GatewayPortSource::Config);
    }

    #[test]
    fn gateway_control_listener_port_prefers_cli_override() {
        let mut env = ScopedEnv::new();
        env.set(GATEWAY_CONTROL_PORT_ENV, "26316");

        let config = LoongConfig::default();
        let resolution =
            resolve_gateway_control_listener_port(&config, Some(26_326)).expect("resolution");

        assert_eq!(resolution.port, 26_326);
        assert_eq!(resolution.source, GatewayPortSource::Cli);
    }

    #[test]
    fn gateway_control_listener_port_accepts_explicit_ephemeral_zero() {
        let mut env = ScopedEnv::new();
        env.remove(GATEWAY_CONTROL_PORT_ENV);

        let config = LoongConfig::default();
        let resolution =
            resolve_gateway_control_listener_port(&config, Some(0)).expect("resolution");

        assert_eq!(resolution.port, 0);
        assert_eq!(resolution.source, GatewayPortSource::EphemeralCli);
    }
}
