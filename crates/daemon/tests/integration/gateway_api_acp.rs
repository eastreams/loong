use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use loong_protocol::ControlPlaneAcpSessionCloseRequest;
use serde_json::Value;
use std::path::PathBuf;
use tower::ServiceExt;

use super::*;

fn gateway_acp_test_config(label: &str, enabled: bool) -> (mvp::config::LoongConfig, PathBuf) {
    let root_dir = unique_temp_dir(label);
    std::fs::create_dir_all(root_dir.as_path()).expect("create gateway ACP test dir");

    let sqlite_path = root_dir.join("memory.sqlite3");
    let sqlite_path_text = sqlite_path.display().to_string();
    let mut config = mvp::config::LoongConfig::default();
    config.acp.enabled = enabled;
    config.memory.sqlite_path = sqlite_path_text;

    (config, root_dir)
}

struct GatewayAcpCloseTestBackend {
    id: &'static str,
}

impl GatewayAcpCloseTestBackend {
    fn new(id: &'static str) -> Self {
        Self { id }
    }
}

#[async_trait]
impl mvp::acp::AcpRuntimeBackend for GatewayAcpCloseTestBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    async fn ensure_session(
        &self,
        _config: &mvp::config::LoongConfig,
        request: &mvp::acp::AcpSessionBootstrap,
    ) -> CliResult<mvp::acp::AcpSessionHandle> {
        Ok(mvp::acp::AcpSessionHandle {
            session_key: request.session_key.clone(),
            backend_id: self.id().to_owned(),
            runtime_session_name: format!("gateway-acp-close-{}", request.session_key),
            working_directory: request.working_directory.clone(),
            backend_session_id: Some(format!("backend-{}", request.session_key)),
            agent_session_id: Some(format!("agent-{}", request.session_key)),
            binding: request.binding.clone(),
        })
    }

    async fn run_turn(
        &self,
        _config: &mvp::config::LoongConfig,
        _session: &mvp::acp::AcpSessionHandle,
        request: &mvp::acp::AcpTurnRequest,
    ) -> CliResult<mvp::acp::AcpTurnResult> {
        Ok(mvp::acp::AcpTurnResult {
            output_text: request.input.clone(),
            state: mvp::acp::AcpSessionState::Ready,
            usage: None,
            events: Vec::new(),
            stop_reason: Some(mvp::acp::AcpTurnStopReason::Completed),
        })
    }

    async fn close(
        &self,
        _config: &mvp::config::LoongConfig,
        _session: &mvp::acp::AcpSessionHandle,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn cancel(
        &self,
        _config: &mvp::config::LoongConfig,
        _session: &mvp::acp::AcpSessionHandle,
    ) -> CliResult<()> {
        Ok(())
    }
}

fn register_gateway_acp_close_test_backend(backend_id: &'static str) {
    mvp::acp::register_acp_backend(backend_id, move || {
        Box::new(GatewayAcpCloseTestBackend::new(backend_id))
    })
    .expect("register gateway ACP close test backend");
}

async fn json_body(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("decode JSON body")
}

#[tokio::test]
async fn gateway_acp_observability_rejects_missing_auth() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-observability-auth", true);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/observability")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_status_returns_service_unavailable_when_acp_disabled() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-status-disabled", false);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/status?session_id=opaque-session")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_dispatch_returns_service_unavailable_when_acp_disabled() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-dispatch-disabled", false);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/dispatch?session_id=opaque-session")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_status_returns_not_found_for_unregistered_session() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-status-missing", true);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/status?session_id=opaque-session")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = json_body(response).await;
    let error_code = body["error"]["code"].as_str().expect("error code");
    let error_message = body["error"]["message"].as_str().expect("error message");
    assert_eq!(error_code, "not_found");
    assert!(error_message.contains("not registered"));

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_observability_returns_snapshot_json() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-observability-ok", true);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/observability")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let active_sessions = body["snapshot"]["runtime_cache"]["active_sessions"]
        .as_u64()
        .expect("active session count");
    assert_eq!(active_sessions, 0);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_dispatch_returns_read_model_payload() {
    let (config, root_dir) = gateway_acp_test_config("gateway-acp-dispatch", true);
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config,
        manager,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/dispatch?session_id=agent%3Acodex%3Aopaque-session&channel_id=telegram&conversation_id=42&account_id=ops-bot&thread_id=thread-1")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let channel_id = body["address"]["channel_id"].as_str().expect("channel id");
    let route_via_acp = body["dispatch"]["decision"]["route_via_acp"]
        .as_bool()
        .expect("route via ACP flag");
    let target_channel_id = body["dispatch"]["decision"]["target"]["channel_id"]
        .as_str()
        .expect("dispatch target channel id");

    assert_eq!(channel_id, "telegram");
    assert!(route_via_acp);
    assert_eq!(target_channel_id, "telegram");

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_acp_close_closes_visible_session_and_returns_shutdown_reason() {
    let (mut config, root_dir) = gateway_acp_test_config("gateway-acp-close", true);
    let backend_id: &'static str = Box::leak(
        format!(
            "gateway-acp-close-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        )
        .into_boxed_str(),
    );
    register_gateway_acp_close_test_backend(backend_id);
    config.acp.backend = Some(backend_id.to_owned());
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    manager
        .ensure_session(
            &config,
            &mvp::acp::AcpSessionBootstrap {
                session_key: "agent:codex:close-me".to_owned(),
                conversation_id: Some("close-me".to_owned()),
                binding: None,
                working_directory: None,
                initial_prompt: None,
                mode: Some(mvp::acp::AcpSessionMode::Interactive),
                mcp_servers: Vec::new(),
                metadata: std::collections::BTreeMap::new(),
            },
        )
        .await
        .expect("ensure ACP session");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config.clone(),
        manager.clone(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/acp/close")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlaneAcpSessionCloseRequest {
                        session_key: Some("agent:codex:close-me".to_owned()),
                        conversation_id: None,
                        route_session_id: None,
                    })
                    .expect("encode ACP close request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["resolved_session_key"], "agent:codex:close-me");
    assert_eq!(body["closed"], true);
    assert_eq!(body["hook_dispatched"], true);
    assert_eq!(body["shutdown_reason"], "explicit_close");
    let status = manager.get_status(&config, "agent:codex:close-me").await;
    assert!(status.is_err(), "session should be removed after close");

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_local_acp_close_closes_visible_session_and_returns_shutdown_reason() {
    let (mut config, root_dir) = gateway_acp_test_config("gateway-local-acp-close", true);
    let backend_id: &'static str = Box::leak(
        format!(
            "gateway-local-acp-close-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        )
        .into_boxed_str(),
    );
    register_gateway_acp_close_test_backend(backend_id);
    config.acp.backend = Some(backend_id.to_owned());
    let manager = mvp::acp::shared_acp_session_manager(&config).expect("shared ACP manager");
    manager
        .ensure_session(
            &config,
            &mvp::acp::AcpSessionBootstrap {
                session_key: "agent:codex:close-me-local".to_owned(),
                conversation_id: Some("close-me-local".to_owned()),
                binding: None,
                working_directory: None,
                initial_prompt: None,
                mode: Some(mvp::acp::AcpSessionMode::Interactive),
                mcp_servers: Vec::new(),
                metadata: std::collections::BTreeMap::new(),
            },
        )
        .await
        .expect("ensure ACP session");
    let app = loong_daemon::gateway::control::build_gateway_acp_test_router(
        "test-token".to_owned(),
        config.clone(),
        manager.clone(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/gateway/acp/close")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlaneAcpSessionCloseRequest {
                        session_key: Some("agent:codex:close-me-local".to_owned()),
                        conversation_id: None,
                        route_session_id: None,
                    })
                    .expect("encode local ACP close request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["resolved_session_key"], "agent:codex:close-me-local");
    assert_eq!(body["closed"], true);
    assert_eq!(body["hook_dispatched"], true);
    assert_eq!(body["shutdown_reason"], "explicit_close");
    let status = manager
        .get_status(&config, "agent:codex:close-me-local")
        .await;
    assert!(
        status.is_err(),
        "session should be removed after local gateway close"
    );

    std::fs::remove_dir_all(root_dir).ok();
}
