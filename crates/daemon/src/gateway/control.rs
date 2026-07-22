use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use loong_protocol::{
    ControlPlaneConnectErrorCode, ControlPlaneConnectErrorResponse, ControlPlaneConnectRequest,
    ControlPlanePrincipal, ControlPlaneScope,
};
use loong_runtime::runtime::Runtime;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinHandle,
};

use crate::mvp::acp::AcpSessionManager;
use crate::mvp::config::LoongConfig;
use crate::{CliResult, mvp, supervisor::LoadedSupervisorConfig};

use super::acp_api::{
    handle_gateway_acp_observability, handle_gateway_acp_sessions, handle_gateway_acp_status,
};
use super::api_acp::{handle_acp_dispatch, handle_acp_observability, handle_acp_status};
use super::api_events::handle_events;
use super::api_health::handle_health;
use super::api_turn::handle_turn;
use super::event_bus::GatewayEventBus;
use super::lifecycle::{
    combine_gateway_control_task_results, gateway_current_time_ms, gateway_stop_outcome_code,
    gateway_stop_outcome_message, gateway_stop_outcome_status, json_error, json_response,
    merge_gateway_control_errors, new_gateway_control_bearer_token,
    remove_gateway_control_token_file, write_gateway_control_token_file,
};
use super::openai_compat::{handle_chat_completions, handle_models};
use super::pairing_api::{
    handle_gateway_nodes, handle_gateway_pairing_complete, handle_gateway_pairing_events,
    handle_gateway_pairing_requests, handle_gateway_pairing_resolve,
    handle_gateway_pairing_session, handle_gateway_pairing_start, handle_gateway_pairing_stream,
};
use super::pairing_runtime::{
    attach_gateway_pairing_runtime_persist_hook, ensure_gateway_pairing_session_scope,
    persist_gateway_pairing_runtime_state, resolve_gateway_pairing_session_lease,
};
use super::read_models::{
    GatewayChannelInventoryReadModel, GatewayChannelInventorySchema,
    GatewayChannelInventorySummaryReadModel, GatewayChannelOperationalModelCountsReadModel,
    GatewayChannelRuntimeKindCountsReadModel, GatewayChannelServiceContractModelCountsReadModel,
    GatewayPairingSessionLeaseReadModel, GatewayRuntimeSnapshotChannelsReadModel,
    GatewayRuntimeSnapshotReadModel, GatewayRuntimeSnapshotSchema,
    GatewayRuntimeSnapshotToolsReadModel,
};
use super::state::{
    GatewayControlSurfaceBinding, gateway_control_token_path, load_gateway_owner_status,
    load_gateway_pairing_runtime_state, request_gateway_stop,
};
use super::status_api::{
    handle_gateway_channels, handle_gateway_operator_summary, handle_gateway_runtime_snapshot,
    handle_gateway_status,
};
use super::support::{
    build_gateway_channel_inventory_read_model, build_gateway_runtime_snapshot_read_model,
    gateway_control_listener_address_from_port_resolution, resolve_gateway_control_listener_port,
    serialize_json_value,
};

const GATEWAY_PAIRING_CHALLENGE_MAX_FUTURE_SKEW_MS: u64 = 30_000;

pub(super) type GatewayControlJsonResponse = (StatusCode, Json<Value>);

pub(super) struct GatewayControlRequest<'a> {
    app_state: &'a GatewayControlAppState,
}

impl<'a> GatewayControlRequest<'a> {
    pub(super) fn authorize(
        headers: &HeaderMap,
        app_state: &'a GatewayControlAppState,
    ) -> Result<Self, GatewayControlJsonResponse> {
        authorize_request_from_state(headers, app_state).map_err(|error| {
            json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str())
        })?;
        Ok(Self { app_state })
    }

    pub(super) fn app_state(&self) -> &'a GatewayControlAppState {
        self.app_state
    }

    pub(super) fn status(
        &self,
    ) -> Result<super::state::GatewayOwnerStatus, GatewayControlJsonResponse> {
        load_gateway_owner_status(self.app_state.runtime_dir.as_path()).ok_or_else(|| {
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "status_unavailable",
                "gateway owner status is unavailable",
            )
        })
    }

    pub(super) fn config(&self) -> Result<&'a LoongConfig, GatewayControlJsonResponse> {
        gateway_control_config(self.app_state).map_err(|error| {
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            )
        })
    }

    pub(super) fn acp_manager(&self) -> Result<&'a AcpSessionManager, GatewayControlJsonResponse> {
        gateway_control_acp_manager(self.app_state).map_err(|error| {
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "acp_unavailable",
                error.as_str(),
            )
        })
    }

    pub(super) fn pairing_registry(
        &self,
    ) -> Result<mvp::control_plane::ControlPlanePairingRegistry, GatewayControlJsonResponse> {
        gateway_pairing_registry(self.app_state).map_err(|error| {
            json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "pairing_unavailable",
                error.as_str(),
            )
        })
    }
}

pub(super) struct GatewayPairingSessionRequest {
    token: String,
    lease: mvp::control_plane::ControlPlaneConnectionLease,
}

impl GatewayPairingSessionRequest {
    pub(super) fn authorize(
        headers: &HeaderMap,
        app_state: &GatewayControlAppState,
        required_scope: ControlPlaneScope,
    ) -> Result<Self, GatewayControlJsonResponse> {
        let token = extract_gateway_pairing_session_token(headers).ok_or_else(|| {
            json_error(
                StatusCode::UNAUTHORIZED,
                "missing_session_token",
                "missing gateway pairing session token",
            )
        })?;
        let lease = resolve_gateway_pairing_session_lease(app_state, token.as_str())?;
        ensure_gateway_pairing_session_scope(&lease, required_scope)?;
        Ok(Self { token, lease })
    }

    pub(super) fn lease(&self) -> &mvp::control_plane::ControlPlaneConnectionLease {
        &self.lease
    }

    pub(super) fn acknowledge_seq(
        mut self,
        app_state: &GatewayControlAppState,
        ack_seq: u64,
    ) -> Result<Self, GatewayControlJsonResponse> {
        let lease = app_state
            .connection_registry
            .acknowledge_seq(self.token.as_str(), ack_seq)
            .map_err(|error| {
                json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session_registry_failed",
                    error.as_str(),
                )
            })?
            .ok_or_else(|| {
                json_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_session_token",
                    "invalid or expired gateway pairing session token",
                )
            })?;
        self.lease = lease;
        Ok(self)
    }
}

#[derive(Clone)]
pub(crate) struct GatewayControlAppState {
    /// One governance/runtime registry shared by every gateway-owned session.
    pub(crate) runtime: Arc<Runtime<mvp::RuntimeContextFactory>>,
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
        let mut config = LoongConfig::default();
        config.audit.mode = mvp::config::AuditMode::InMemory;
        Self::test_state(bearer_token, config, None)
    }

    /// Test state whose Runtime and exposed config are one atomic snapshot.
    pub fn test_with_config(bearer_token: String, mut config: LoongConfig) -> Self {
        config.audit.mode = mvp::config::AuditMode::InMemory;
        Self::test_state(bearer_token, config.clone(), Some(config))
    }

    // Centralizing fixture assembly prevents tests from replacing config after
    // Runtime bootstrap, a state production construction cannot represent.
    fn test_state(
        bearer_token: String,
        runtime_config: LoongConfig,
        config: Option<LoongConfig>,
    ) -> Self {
        let runtime = mvp::runtime::bootstrap_runtime_with_config(&runtime_config)
            .expect("minimal gateway test runtime should bootstrap");
        let channel_inventory = minimal_gateway_channel_inventory_read_model();
        let runtime_snapshot =
            minimal_gateway_runtime_snapshot_read_model(channel_inventory.clone());
        Self {
            runtime,
            runtime_dir: PathBuf::from("/tmp/test"),
            config_path: String::new(),
            bearer_token,
            channel_inventory: Arc::new(channel_inventory),
            runtime_snapshot: Arc::new(runtime_snapshot),
            event_bus: None,
            acp_manager: None,
            challenge_registry: Arc::new(mvp::control_plane::ControlPlaneChallengeRegistry::new()),
            connection_registry: Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new()),
            config,
        }
    }
}

fn minimal_gateway_channel_inventory_read_model() -> GatewayChannelInventoryReadModel {
    GatewayChannelInventoryReadModel {
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
            runtime_kind_counts: GatewayChannelRuntimeKindCountsReadModel {
                runtime_backed: 0,
                plugin_backed: 0,
                outbound_only: 0,
                catalog_only: 0,
            },
            operational_model_counts: GatewayChannelOperationalModelCountsReadModel {
                gateway_supervised: 0,
                standalone_runtime: 0,
                plugin_backed: 0,
                outbound_only: 0,
                catalog_only: 0,
            },
            service_contract_model_counts: GatewayChannelServiceContractModelCountsReadModel {
                managed_bridge_capable_service: 0,
                native_service_channel: 0,
                standalone_native_service: 0,
                external_plugin_bridge: 0,
                direct_send_only: 0,
                catalog_only: 0,
            },
        },
        channels: vec![],
        catalog_only_channels: vec![],
        channel_catalog: vec![],
        channel_surfaces: vec![],
        channel_access_policies: vec![],
    }
}

fn minimal_gateway_tool_runtime_read_model() -> GatewayRuntimeSnapshotToolsReadModel {
    GatewayRuntimeSnapshotToolsReadModel {
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
        access: super::read_models::GatewayToolAccessReadModel {
            ordinary_network_access_enabled: false,
            query_search_enabled: false,
            query_search_default_provider: "duckduckgo".to_owned(),
            query_search_source: "external_provider".to_owned(),
            query_search_provider_label: "DuckDuckGo".to_owned(),
            query_search_credential_ready: true,
            browser_page_access_enabled: false,
            managed_browser_session_enabled: false,
            managed_browser_session_ready: false,
            consent_mode: "full".to_owned(),
            approval_mode: "disabled".to_owned(),
            separation_note: crate::RUNTIME_TOOL_ACCESS_SEPARATION_NOTE.to_owned(),
        },
    }
}

fn minimal_gateway_runtime_snapshot_read_model(
    channel_inventory: GatewayChannelInventoryReadModel,
) -> GatewayRuntimeSnapshotReadModel {
    use serde_json::json;

    GatewayRuntimeSnapshotReadModel {
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
            inventory: channel_inventory,
        },
        tool_runtime: json!({}),
        tools: minimal_gateway_tool_runtime_read_model(),
        runtime_plugins: json!({}),
        skills: json!({}),
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
    // Runtime is a gateway-lifetime owner. Sessions borrow it through Context;
    // individual HTTP turns and channel accounts must never bootstrap another.
    let runtime = mvp::runtime::bootstrap_runtime_with_config(&loaded_config.config)?;
    let channel_inventory = build_gateway_channel_inventory_read_model(loaded_config)?;
    let runtime_snapshot = build_gateway_runtime_snapshot_read_model(loaded_config)?;
    let bearer_token = new_gateway_control_bearer_token();
    let token_path = gateway_control_token_path(runtime_dir);
    let persisted_pairing_runtime = load_gateway_pairing_runtime_state(runtime_dir);

    write_gateway_control_token_file(token_path.as_path(), bearer_token.as_str())?;
    let (listener, binding) =
        bind_gateway_control_listener(loaded_config, port_override, token_path.as_path()).await?;
    let (gateway_ingress_router, gateway_ingress_runtimes) =
        build_gateway_control_ingress(loaded_config, Arc::clone(&runtime), token_path.as_path())
            .await?;

    let connection_registry =
        build_gateway_pairing_connection_registry(persisted_pairing_runtime.as_ref())?;
    let event_bus =
        build_gateway_pairing_event_bus(persisted_pairing_runtime.as_ref(), acp_manager.is_some());

    let app_state = GatewayControlAppState {
        runtime,
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
    let router = build_gateway_control_router(app_state).merge(gateway_ingress_router);

    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let (exit_sender, _) = watch::channel::<Option<CliResult<()>>>(None);
    let exit_sender_for_task = exit_sender.clone();
    let token_path_for_task = token_path;
    let gateway_ingress_runtimes_for_task = gateway_ingress_runtimes;
    let join_handle = tokio::spawn(async move {
        let server = axum::serve(listener, router);
        let server = server.with_graceful_shutdown(async move {
            let _ = shutdown_receiver.await;
        });
        let server_result = server
            .await
            .map_err(|error| format!("gateway control surface server failed: {error}"));
        let ingress_shutdown_result =
            mvp::channel::shutdown_gateway_ingress_runtimes(gateway_ingress_runtimes_for_task)
                .await;
        let server_result =
            combine_gateway_control_task_results(server_result, ingress_shutdown_result);
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

async fn bind_gateway_control_listener(
    loaded_config: &LoadedSupervisorConfig,
    port_override: Option<u16>,
    token_path: &Path,
) -> CliResult<(TcpListener, GatewayControlSurfaceBinding)> {
    let port_resolution =
        resolve_gateway_control_listener_port(&loaded_config.config, port_override)?;
    let listener_address = gateway_control_listener_address_from_port_resolution(port_resolution);
    let listener = TcpListener::bind(listener_address).await.map_err(|error| {
        let bind_error = format!("bind gateway control surface failed: {error}");
        let cleanup_result = remove_gateway_control_token_file(token_path);
        merge_gateway_control_errors(bind_error, cleanup_result.err())
    })?;
    let local_address = listener.local_addr().map_err(|error| {
        let address_error = format!("read gateway control surface local address failed: {error}");
        let cleanup_result = remove_gateway_control_token_file(token_path);
        merge_gateway_control_errors(address_error, cleanup_result.err())
    })?;
    let binding = GatewayControlSurfaceBinding {
        bind_address: local_address.ip().to_string(),
        port: local_address.port(),
        port_source: port_resolution.source,
        token_path: token_path.to_path_buf(),
    };
    Ok((listener, binding))
}

async fn build_gateway_control_ingress(
    loaded_config: &LoadedSupervisorConfig,
    runtime: Arc<Runtime<mvp::RuntimeContextFactory>>,
    token_path: &Path,
) -> CliResult<(
    axum::Router,
    Vec<Arc<crate::mvp::channel::ChannelOperationRuntimeTracker>>,
)> {
    mvp::channel::build_gateway_ingress(
        loaded_config.resolved_path.as_path(),
        &loaded_config.config,
        runtime,
    )
    .await
    .map(|gateway_ingress| gateway_ingress.into_parts())
    .map_err(|error| {
        let cleanup_result = remove_gateway_control_token_file(token_path);
        merge_gateway_control_errors(error, cleanup_result.err())
    })
}

fn build_gateway_pairing_connection_registry(
    persisted_pairing_runtime: Option<&super::state::GatewayPairingRuntimeState>,
) -> CliResult<Arc<mvp::control_plane::ControlPlaneConnectionRegistry>> {
    let connection_registry = Arc::new(mvp::control_plane::ControlPlaneConnectionRegistry::new());
    if let Some(persisted_pairing_runtime) = persisted_pairing_runtime {
        connection_registry
            .restore_leases(&persisted_pairing_runtime.sessions)
            .map_err(|error| format!("restore gateway pairing sessions failed: {error}"))?;
    }
    Ok(connection_registry)
}

fn build_gateway_pairing_event_bus(
    persisted_pairing_runtime: Option<&super::state::GatewayPairingRuntimeState>,
    acp_enabled: bool,
) -> Option<GatewayEventBus> {
    match (persisted_pairing_runtime, acp_enabled) {
        (Some(persisted_pairing_runtime), _) => Some(GatewayEventBus::from_snapshot(
            256,
            persisted_pairing_runtime.event_bus.clone(),
        )),
        (None, true) => Some(GatewayEventBus::new(256)),
        (None, false) => None,
    }
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
        .route("/api/gateway/nodes", get(handle_gateway_nodes))
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
        .route("/v1/nodes", get(handle_gateway_nodes))
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

async fn handle_gateway_stop(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(response) = GatewayControlRequest::authorize(&headers, app_state.as_ref()) {
        return response;
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

pub(super) fn authorize_request(headers: &HeaderMap, expected_token: &str) -> CliResult<()> {
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

pub(super) fn gateway_pairing_registry(
    app_state: &GatewayControlAppState,
) -> CliResult<mvp::control_plane::ControlPlanePairingRegistry> {
    let config = gateway_control_config(app_state)?;
    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config =
            crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
                &config.memory,
            );
        let session_store_config =
            crate::mvp::session::store::SessionStoreConfig::from(&memory_config);
        mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(session_store_config)
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = config;
        Err("gateway pairing requires sqlite memory support".to_owned())
    }
}

pub(super) fn issue_gateway_pairing_session_lease(
    app_state: &GatewayControlAppState,
    request: &ControlPlaneConnectRequest,
) -> GatewayPairingSessionLeaseReadModel {
    let connection_id = format!(
        "gwp-{:016x}",
        gateway_current_time_ms().saturating_add(rand::random::<u32>() as u64)
    );
    let principal = crate::control_plane_device_auth::connection_principal_from_connect_request(
        request,
        connection_id,
        &request.scopes,
    );
    let lease = app_state.connection_registry.issue(principal);
    let principal = gateway_pairing_protocol_principal(&lease);
    GatewayPairingSessionLeaseReadModel {
        connection_token: lease.token,
        connection_token_expires_at_ms: lease.expires_at_ms,
        principal,
        last_acknowledged_seq: lease.acknowledged_seq,
    }
}

pub(super) fn gateway_pairing_protocol_principal(
    lease: &mvp::control_plane::ControlPlaneConnectionLease,
) -> ControlPlanePrincipal {
    crate::control_plane_device_auth::protocol_principal_from_connection_lease(lease)
}

pub(super) fn extract_gateway_pairing_session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-loong-pairing-session-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

pub(super) fn verify_gateway_pairing_device_challenge(
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

    crate::control_plane_device_auth::validate_control_plane_device_challenge(
        request,
        &challenge,
        GATEWAY_PAIRING_CHALLENGE_MAX_FUTURE_SKEW_MS,
        crate::control_plane_device_auth::current_time_ms(),
    )
    .map_err(|error| {
        let (status, code) =
            if error.starts_with("control-plane device signature verification failed:") {
                (
                    StatusCode::UNAUTHORIZED,
                    ControlPlaneConnectErrorCode::DeviceSignatureInvalid,
                )
            } else if error.starts_with("invalid control-plane device public_key")
                || error.starts_with("invalid control-plane device signature")
                || error == "control-plane device public_key must decode to 32 bytes"
            {
                (
                    StatusCode::BAD_REQUEST,
                    ControlPlaneConnectErrorCode::DeviceSignatureInvalid,
                )
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    ControlPlaneConnectErrorCode::ChallengeExpired,
                )
            };
        json_connect_error(status, code, error)
    })?;

    Ok(())
}

pub(super) fn json_connect_error(
    status_code: StatusCode,
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> GatewayControlJsonResponse {
    json_connect_error_with_request(status_code, code, error, None)
}

pub(super) fn json_connect_error_with_request(
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

pub(super) fn gateway_control_payload_response<T: Serialize>(
    value: &T,
    context: &str,
) -> GatewayControlJsonResponse {
    match serialize_json_value(value, context) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(error) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            error.as_str(),
        ),
    }
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
    let mut state = GatewayControlAppState::test_with_config(bearer_token, config);
    state.acp_manager = Some(acp_manager);
    let app_state = Arc::new(state);
    Router::new()
        .route("/v1/acp/status", get(handle_acp_status))
        .route("/v1/acp/observability", get(handle_acp_observability))
        .route("/v1/acp/dispatch", get(handle_acp_dispatch))
        .with_state(app_state)
}

/// Minimal router for gateway pairing endpoint integration tests.
#[doc(hidden)]
pub fn build_gateway_pairing_test_router_without_event_bus(
    bearer_token: String,
    config: LoongConfig,
) -> Router {
    let state = GatewayControlAppState::test_with_config(bearer_token, config);
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
    let mut state = GatewayControlAppState::test_with_config(bearer_token, config);
    state.event_bus = Some(event_bus);
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

/// Minimal router for gateway node inventory integration tests.
#[doc(hidden)]
pub fn build_gateway_nodes_test_router(
    bearer_token: String,
    config: LoongConfig,
    channel_inventory: GatewayChannelInventoryReadModel,
) -> Router {
    let mut state = GatewayControlAppState::test_with_config(bearer_token, config);
    state.channel_inventory = Arc::new(channel_inventory);
    let app_state = Arc::new(state);
    Router::new()
        .route("/v1/nodes", get(handle_gateway_nodes))
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::{
        gateway_control_listener_address_from_port_resolution,
        resolve_gateway_control_listener_port,
    };
    use crate::gateway::state::GatewayPortSource;
    use crate::gateway::support::GATEWAY_CONTROL_PORT_ENV;
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
