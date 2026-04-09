use super::*;

use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
};
use loongclaw_daemon::{
    CliResult,
    gateway::{
        client::{GatewayAcpSessionsRequest, GatewayAcpStatusRequest, GatewayLocalClient},
        service::run_gateway_run_with_hooks_for_test,
        state::{load_gateway_owner_status, request_gateway_stop},
    },
    mvp::{
        acp::{
            AcpBackendMetadata, AcpCapability, AcpRuntimeBackend, AcpSessionBootstrap,
            AcpSessionHandle, AcpSessionState, AcpSessionStore, AcpSqliteSessionStore,
            AcpTurnRequest, AcpTurnResult, AcpTurnStopReason, register_acp_backend,
        },
        config::{AcpConfig, LoongClawConfig},
    },
    supervisor::{LoadedSupervisorConfig, SupervisorRuntimeHooks},
};
use serde_json::json;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;

type BoxedShutdownFuture = Pin<Box<dyn Future<Output = CliResult<String>> + Send + 'static>>;

const GATEWAY_TURN_TEST_TIMEOUT: Duration = Duration::from_secs(2);
const GATEWAY_CONTROL_SURFACE_WAIT_ATTEMPTS: usize = 400;
const GATEWAY_CONTROL_SURFACE_WAIT_INTERVAL: Duration = Duration::from_millis(10);

struct GatewayEchoBackend {
    id: &'static str,
}

#[async_trait]
impl AcpRuntimeBackend for GatewayEchoBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    fn metadata(&self) -> AcpBackendMetadata {
        let capabilities = [
            AcpCapability::SessionLifecycle,
            AcpCapability::TurnExecution,
        ];
        let description = "Gateway integration echo backend";
        AcpBackendMetadata::new(self.id(), capabilities, description)
    }

    async fn ensure_session(
        &self,
        _config: &LoongClawConfig,
        request: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionHandle> {
        let session_key = request.session_key.clone();
        let backend_id = self.id().to_owned();
        let runtime_session_name = format!("runtime-{session_key}");
        let working_directory = request.working_directory.clone();
        let backend_session_id = Some(format!("backend-{session_key}"));
        let agent_session_id = Some(format!("agent-{session_key}"));
        let binding = request.binding.clone();

        Ok(AcpSessionHandle {
            session_key,
            backend_id,
            runtime_session_name,
            working_directory,
            backend_session_id,
            agent_session_id,
            binding,
        })
    }

    async fn run_turn(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
        request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult> {
        let output_text = format!("echo: {}", request.input);
        let state = AcpSessionState::Ready;
        let usage = None;
        let events = Vec::new();
        let stop_reason = Some(AcpTurnStopReason::Completed);

        Ok(AcpTurnResult {
            output_text,
            state,
            usage,
            events,
            stop_reason,
        })
    }

    async fn cancel(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn close(&self, _config: &LoongClawConfig, _session: &AcpSessionHandle) -> CliResult<()> {
        Ok(())
    }
}

fn pending_shutdown_future() -> BoxedShutdownFuture {
    Box::pin(async move {
        std::future::pending::<()>().await;
        Ok(String::new())
    })
}

fn register_gateway_echo_backend(prefix: &str) -> &'static str {
    let temp_dir = super::unique_temp_dir(prefix);
    let directory_name = temp_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(prefix);
    let backend_name = format!("{prefix}-{directory_name}");
    let backend_id: &'static str = Box::leak(backend_name.into_boxed_str());
    let backend_id_for_factory = backend_id;
    let backend_factory = move || {
        let backend = GatewayEchoBackend {
            id: backend_id_for_factory,
        };
        let backend = Box::new(backend);
        backend as Box<dyn AcpRuntimeBackend>
    };
    let register_result = register_acp_backend(backend_id, backend_factory);
    register_result.expect("register gateway echo backend");

    backend_id
}

fn unique_sqlite_path(label: &str) -> PathBuf {
    let sqlite_dir = super::unique_temp_dir(label);
    let create_dir_result = std::fs::create_dir_all(sqlite_dir.as_path());
    create_dir_result.expect("create sqlite temp dir");
    sqlite_dir.join("gateway.sqlite3")
}

fn gateway_config_path(sqlite_path: &Path) -> PathBuf {
    let parent_directory = sqlite_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    parent_directory.join("loongclaw-gateway-acp.toml")
}

fn gateway_turn_loaded_config_fixture(
    sqlite_path: &Path,
    backend_id: &str,
) -> LoadedSupervisorConfig {
    let mut config = LoongClawConfig::default();
    let sqlite_path_text = sqlite_path.display().to_string();
    config.memory.sqlite_path = sqlite_path_text;
    config.acp = AcpConfig {
        enabled: true,
        backend: Some(backend_id.to_owned()),
        ..AcpConfig::default()
    };
    let resolved_path = gateway_config_path(sqlite_path);

    LoadedSupervisorConfig {
        resolved_path,
        config,
    }
}

async fn wait_for_gateway_control_surface(runtime_dir: &Path) {
    for _ in 0..GATEWAY_CONTROL_SURFACE_WAIT_ATTEMPTS {
        let maybe_status = load_gateway_owner_status(runtime_dir);

        if let Some(status) = maybe_status {
            let failed_phase = status.phase == "failed";
            if failed_phase {
                let error_message = status
                    .last_error
                    .unwrap_or_else(|| "unknown gateway owner failure".to_owned());
                panic!("gateway owner failed before control surface binding: {error_message}");
            }

            let has_bind_address = status.bind_address.is_some();
            let has_port = status.port.is_some();
            let has_token_path = status.token_path.is_some();

            if status.running && has_bind_address && has_port && has_token_path {
                return;
            }
        }

        sleep(GATEWAY_CONTROL_SURFACE_WAIT_INTERVAL).await;
    }

    panic!("timed out waiting for gateway control surface binding");
}

#[tokio::test]
async fn gateway_turn_rejects_missing_auth() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({"session_id": "s1", "input": "hello"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_turn_rejects_missing_session_id() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({"input": "hello"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer tok")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn gateway_turn_rejects_empty_input() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({"session_id": "s1", "input": ""});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer tok")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn gateway_turn_returns_503_when_no_acp_backend() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({"session_id": "s1", "input": "hello"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer tok")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn gateway_turn_rejects_channel_scope_without_conversation_id() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({
        "session_id": "opaque-session",
        "channel_id": "telegram",
        "input": "hello"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer tok")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn gateway_turn_accepts_structured_session_scope_before_backend_check() {
    let app = loongclaw_daemon::gateway::api_turn::build_turn_test_router_no_backend("tok".into());
    let body = json!({
        "session_id": "opaque-session",
        "channel_id": "telegram",
        "conversation_id": "42",
        "account_id": "ops-bot",
        "thread_id": "thread-1",
        "input": "hello"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/turn")
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, "Bearer tok")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_run_turn_persists_acp_session_metadata_into_configured_sqlite_store() {
    let backend_id = register_gateway_echo_backend("gateway-turn");
    let sqlite_path = unique_sqlite_path("gateway-turn-store");
    let runtime_dir = super::unique_temp_dir("gateway-turn-runtime");
    let runtime_dir_for_run = runtime_dir.clone();
    let sqlite_path_for_config = sqlite_path.clone();
    let backend_id_for_config = backend_id.to_owned();

    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(move |_| {
            let config = gateway_turn_loaded_config_fixture(
                sqlite_path_for_config.as_path(),
                backend_id_for_config.as_str(),
            );
            Ok(config)
        }),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let run = tokio::spawn(async move {
        run_gateway_run_with_hooks_for_test(
            None,
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_for_gateway_control_surface(runtime_dir.as_path()).await;

    let client_result = GatewayLocalClient::discover(runtime_dir.as_path());
    let client = client_result.expect("discover gateway client");
    let turn_response = client
        .turn("gateway-session", "hello through gateway")
        .await
        .expect("gateway turn should succeed");

    assert_eq!(turn_response["output_text"], "echo: hello through gateway");

    let store = AcpSqliteSessionStore::new(Some(sqlite_path.clone()));
    let session_key = "agent:codex:gateway-session";
    let persisted_result = AcpSessionStore::get(&store, session_key);
    let persisted_option = persisted_result.expect("read persisted ACP session metadata");
    let persisted =
        persisted_option.expect("gateway ACP session metadata should persist into sqlite");

    assert_eq!(persisted.session_key, "agent:codex:gateway-session");
    assert_eq!(persisted.backend_id, backend_id);
    assert_eq!(
        persisted.conversation_id.as_deref(),
        Some("gateway-session")
    );

    request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");

    let supervisor = timeout(GATEWAY_TURN_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop")
        .expect("join gateway run")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn gateway_acp_operator_endpoints_surface_shared_session_truth() {
    let backend_id = register_gateway_echo_backend("gateway-acp-surface");
    let sqlite_path = unique_sqlite_path("gateway-acp-surface-store");
    let config_path = gateway_config_path(sqlite_path.as_path());
    let runtime_dir = super::unique_temp_dir("gateway-acp-surface-runtime");
    let runtime_dir_for_run = runtime_dir.clone();
    let sqlite_path_for_config = sqlite_path.clone();
    let backend_id_for_config = backend_id.to_owned();

    let hooks = SupervisorRuntimeHooks {
        load_config: Arc::new(move |_| {
            let config = gateway_turn_loaded_config_fixture(
                sqlite_path_for_config.as_path(),
                backend_id_for_config.as_str(),
            );
            Ok(config)
        }),
        initialize_runtime_environment: Arc::new(|_| {}),
        run_cli_host: Arc::new(|_| {
            panic!("headless gateway run should not start the concurrent CLI host")
        }),
        background_channel_runners: BTreeMap::new(),
        wait_for_shutdown: Arc::new(pending_shutdown_future),
        observe_state: Arc::new(|_| Ok(())),
    };

    let run = tokio::spawn(async move {
        run_gateway_run_with_hooks_for_test(
            None,
            None,
            Vec::new(),
            runtime_dir_for_run.as_path(),
            hooks,
        )
        .await
    });

    wait_for_gateway_control_surface(runtime_dir.as_path()).await;

    let client_result = GatewayLocalClient::discover(runtime_dir.as_path());
    let client = client_result.expect("discover gateway client");
    let turn_result = client
        .turn("gateway-session", "operator truth")
        .await
        .expect("gateway turn should succeed");

    assert_eq!(turn_result["output_text"], "echo: operator truth");

    let sessions_request = GatewayAcpSessionsRequest { limit: Some(10) };
    let sessions = client
        .acp_sessions(&sessions_request)
        .await
        .expect("read gateway ACP sessions");

    assert_eq!(sessions["matched_count"].as_u64(), Some(1));
    assert_eq!(sessions["returned_count"].as_u64(), Some(1));
    let sessions_array = sessions["sessions"]
        .as_array()
        .expect("gateway ACP sessions array");
    assert_eq!(sessions_array.len(), 1);
    let session = sessions_array.first().expect("gateway ACP session");
    assert_eq!(session["session_key"], "agent:codex:gateway-session");
    assert_eq!(session["backend_id"], backend_id);
    assert_eq!(session["conversation_id"], "gateway-session");

    let status_request = GatewayAcpStatusRequest {
        session: None,
        conversation_id: Some("gateway-session"),
        route_session_id: None,
    };
    let status = client
        .acp_status(&status_request)
        .await
        .expect("read gateway ACP status");

    assert_eq!(
        status["resolved_session_key"],
        "agent:codex:gateway-session"
    );
    assert_eq!(status["status"]["backend_id"], backend_id);
    assert_eq!(status["status"]["state"], "ready");

    let observability = client
        .acp_observability()
        .await
        .expect("read gateway ACP observability");

    assert_eq!(observability["config"], config_path.display().to_string());
    let active_sessions = observability["snapshot"]["runtime_cache"]["active_sessions"].as_u64();
    assert!(active_sessions.is_some());
    let errors_by_code = observability["snapshot"]["errors_by_code"].as_object();
    assert!(errors_by_code.is_some());

    request_gateway_stop(runtime_dir.as_path()).expect("request gateway stop");

    let supervisor = timeout(GATEWAY_TURN_TEST_TIMEOUT, run)
        .await
        .expect("gateway run should stop")
        .expect("join gateway run")
        .expect("gateway run should return supervisor state");
    assert!(supervisor.final_exit_result().is_ok());
}
