use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
    let app = loongclaw_daemon::gateway::control::build_gateway_acp_test_router(
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
