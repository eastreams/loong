use std::collections::BTreeSet;
use std::path::PathBuf;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use loongclaw_protocol::{
    ControlPlanePairingListResponse, ControlPlanePairingResolveRequest,
    ControlPlanePairingResolveResponse, ControlPlanePairingStatus,
};
use tower::ServiceExt;

use super::*;

fn gateway_pairing_test_config(label: &str) -> (mvp::config::LoongClawConfig, PathBuf) {
    let root_dir = unique_temp_dir(label);
    std::fs::create_dir_all(root_dir.as_path()).expect("create gateway pairing test dir");

    let sqlite_path = root_dir.join("memory.sqlite3");
    let sqlite_path_text = sqlite_path.display().to_string();
    let mut config = mvp::config::LoongClawConfig::default();
    config.memory.sqlite_path = sqlite_path_text;

    (config, root_dir)
}

fn seed_pending_pairing_request(config: &mvp::config::LoongClawConfig) -> String {
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let registry =
        mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(memory_config)
            .expect("pairing registry");
    let requested_scopes = BTreeSet::from(["operator.read".to_owned()]);
    let decision = registry
        .evaluate_connect(
            "device-1",
            "cli",
            "public-key-1",
            "operator",
            &requested_scopes,
            None,
        )
        .expect("evaluate connect");
    match decision {
        mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
            request, ..
        } => request.pairing_request_id,
        other => panic!("expected pending pairing request, got {other:?}"),
    }
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("decode json")
}

#[tokio::test]
async fn gateway_pairing_requests_reject_missing_auth() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-auth");
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/requests")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_requests_return_pending_request_records() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-list");
    let pairing_request_id = seed_pending_pairing_request(&config);
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/requests?status=pending&limit=10")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let list: ControlPlanePairingListResponse =
        serde_json::from_slice(&body).expect("pairing list json");

    assert_eq!(list.matched_count, 1);
    assert_eq!(list.returned_count, 1);
    assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
    assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Pending);
    assert_eq!(list.requests[0].device_id, "device-1");

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_resolve_approves_request_and_returns_device_token() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-resolve");
    let pairing_request_id = seed_pending_pairing_request(&config);
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/resolve")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id,
                        approve: true,
                    })
                    .expect("encode pairing resolve request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolved: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&body).expect("pairing resolve json");

    assert_eq!(resolved.request.status, ControlPlanePairingStatus::Approved);
    assert!(resolved.device_token.is_some());

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_resolve_returns_not_found_for_unknown_request() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-missing");
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/resolve")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&ControlPlanePairingResolveRequest {
                        pairing_request_id: "missing-request".to_owned(),
                        approve: true,
                    })
                    .expect("encode pairing resolve request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "pairing_not_found");

    std::fs::remove_dir_all(root_dir).ok();
}
