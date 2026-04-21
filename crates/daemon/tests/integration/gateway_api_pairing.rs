use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use loong_daemon::mvp::config::LoongConfig;
use loong_protocol::{
    ControlPlanePairingListResponse, ControlPlanePairingResolveRequest,
    ControlPlanePairingResolveResponse, ControlPlanePairingStatus,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use tower::ServiceExt;

use super::*;

fn gateway_pairing_test_config(label: &str) -> (LoongConfig, PathBuf) {
    let root_dir = unique_temp_dir(label);
    std::fs::create_dir_all(root_dir.as_path()).expect("create gateway pairing test dir");

    let sqlite_path = root_dir.join("memory.sqlite3");
    let sqlite_path_text = sqlite_path.display().to_string();
    let mut config = LoongConfig::default();
    config.memory.sqlite_path = sqlite_path_text;

    (config, root_dir)
}

async fn decode_json<T: serde::de::DeserializeOwned>(response: axum::response::Response) -> T {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("decode JSON body")
}

fn seed_pending_pairing_request(config: &LoongConfig) -> String {
    let memory_config =
        loong_daemon::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
            &config.memory,
        );
    let registry =
        loong_daemon::mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(
            memory_config,
        )
        .expect("pairing registry");
    let requested_scopes = BTreeSet::from(["operator.read".to_owned()]);
    let decision = registry.evaluate_connect(
        "device-1",
        "cli-test",
        "pubkey-1",
        "operator",
        &requested_scopes,
        None,
    );
    let decision = decision.expect("evaluate connect");
    match decision {
        loong_daemon::mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
            request,
            ..
        } => request.pairing_request_id,
        other => panic!("expected pending pairing request, got {other:?}"),
    }
}

#[tokio::test]
async fn gateway_pairing_requests_reject_missing_auth() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-auth");
    let app = loong_daemon::gateway::control::build_gateway_pairing_test_router(
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
async fn gateway_pairing_requests_and_resolve_roundtrip_pending_request() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-roundtrip");
    let pairing_request_id = seed_pending_pairing_request(&config);
    let app = loong_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/requests?status=pending&limit=10")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);
    let list: ControlPlanePairingListResponse = decode_json(list_response).await;
    assert_eq!(list.matched_count, 1);
    assert_eq!(list.returned_count, 1);
    assert_eq!(list.requests[0].pairing_request_id, pairing_request_id);
    assert_eq!(list.requests[0].status, ControlPlanePairingStatus::Pending);

    let resolve_request = ControlPlanePairingResolveRequest {
        pairing_request_id: pairing_request_id.clone(),
        approve: true,
    };
    let resolve_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/pairing/resolve")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&resolve_request).expect("encode resolve request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolved: ControlPlanePairingResolveResponse = decode_json(resolve_response).await;
    assert_eq!(resolved.request.pairing_request_id, pairing_request_id);
    assert_eq!(resolved.request.status, ControlPlanePairingStatus::Approved);
    assert!(
        resolved.device_token.is_some(),
        "approved pairing should return a device token"
    );

    let approved_list_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/requests?status=approved&limit=10")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(approved_list_response.status(), StatusCode::OK);
    let approved_list: ControlPlanePairingListResponse = decode_json(approved_list_response).await;
    assert_eq!(approved_list.matched_count, 1);
    assert_eq!(
        approved_list.requests[0].pairing_request_id,
        pairing_request_id
    );
    assert_eq!(
        approved_list.requests[0].status,
        ControlPlanePairingStatus::Approved
    );

    std::fs::remove_dir_all(root_dir).ok();
}
