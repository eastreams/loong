use axum::{
    body::Body,
    http::{
        Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
};
use serde_json::json;
use tower::ServiceExt;

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
