use axum::{
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use tower::ServiceExt;

#[tokio::test]
async fn gateway_sse_stream_returns_event_stream_content_type() {
    let event_bus = loong_daemon::gateway::event_bus::GatewayEventBus::new(64);
    let app = loong_daemon::gateway::control::build_gateway_events_test_router(
        "test-token".to_string(),
        event_bus,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/events")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("text/event-stream"));
}

#[tokio::test]
async fn gateway_sse_rejects_missing_auth() {
    let event_bus = loong_daemon::gateway::event_bus::GatewayEventBus::new(64);
    let app = loong_daemon::gateway::control::build_gateway_events_test_router(
        "test-token".to_string(),
        event_bus,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
