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
    extract::State,
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinHandle,
};

use crate::{
    CliResult, build_channels_cli_json_payload, build_runtime_snapshot_cli_json_payload,
    collect_runtime_snapshot_cli_state_from_loaded_config, mvp, supervisor::LoadedSupervisorConfig,
};

use super::state::{
    GatewayControlSurfaceBinding, GatewayStopRequestOutcome, gateway_control_token_path,
    load_gateway_owner_status, request_gateway_stop,
};

const GATEWAY_CONTROL_TOKEN_FILE_MODE: u32 = 0o600;
const GATEWAY_CONTROL_RUNTIME_DIR_MODE: u32 = 0o700;

type GatewayControlJsonResponse = (StatusCode, Json<Value>);

#[derive(Clone)]
struct GatewayControlAppState {
    runtime_dir: PathBuf,
    bearer_token: String,
    channels_payload: Arc<Value>,
    runtime_snapshot_payload: Arc<Value>,
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
) -> CliResult<GatewayControlSurface> {
    let channels_payload = build_gateway_channels_payload(loaded_config)?;
    let runtime_snapshot_payload = build_gateway_runtime_snapshot_payload(loaded_config)?;
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

    let app_state = GatewayControlAppState {
        runtime_dir: runtime_dir.to_path_buf(),
        bearer_token,
        channels_payload: Arc::new(channels_payload),
        runtime_snapshot_payload: Arc::new(runtime_snapshot_payload),
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
        .route("/api/gateway/stop", post(handle_gateway_stop))
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

    let payload = app_state.channels_payload.as_ref().clone();
    json_response(StatusCode::OK, payload)
}

async fn handle_gateway_runtime_snapshot(
    headers: HeaderMap,
    State(app_state): State<Arc<GatewayControlAppState>>,
) -> GatewayControlJsonResponse {
    if let Err(error) = authorize_request(&headers, app_state.bearer_token.as_str()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized", error.as_str());
    }

    let payload = app_state.runtime_snapshot_payload.as_ref().clone();
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

    if provided_token != expected_token {
        return Err("invalid gateway bearer token".to_owned());
    }

    Ok(())
}

fn build_gateway_channels_payload(loaded_config: &LoadedSupervisorConfig) -> CliResult<Value> {
    let config_path = loaded_config.resolved_path.display().to_string();
    let inventory = mvp::channel::channel_inventory(&loaded_config.config);
    let payload = build_channels_cli_json_payload(config_path.as_str(), &inventory);
    serialize_json_value(&payload, "gateway channels payload")
}

fn build_gateway_runtime_snapshot_payload(
    loaded_config: &LoadedSupervisorConfig,
) -> CliResult<Value> {
    let snapshot = collect_runtime_snapshot_cli_state_from_loaded_config(loaded_config)?;
    build_runtime_snapshot_cli_json_payload(&snapshot)
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

    let open_result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path);
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
