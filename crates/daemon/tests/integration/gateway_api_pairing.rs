use std::collections::BTreeSet;
use std::path::PathBuf;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use loongclaw_protocol::{
    CONTROL_PLANE_PROTOCOL_VERSION, ControlPlaneClientIdentity, ControlPlaneConnectRequest,
    ControlPlaneDeviceIdentity, ControlPlanePairingListResponse,
    ControlPlanePairingResolveRequest, ControlPlanePairingResolveResponse, ControlPlanePairingStatus,
    ControlPlaneRole, ControlPlaneScope,
};
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
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

fn pairing_complete_signature_message(
    request: &ControlPlaneConnectRequest,
    device: &ControlPlaneDeviceIdentity,
) -> Vec<u8> {
    let scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "loongclaw-control-plane-connect-v1\nnonce={}\ndevice_id={}\nclient_id={}\nrole={}\nscopes={}\nsigned_at_ms={}",
        device.nonce,
        device.device_id,
        request.client.id,
        request.role.as_str(),
        scopes,
        device.signed_at_ms
    )
    .into_bytes()
}

fn build_signed_pairing_complete_request(
    signing_key: &SigningKey,
    nonce: &str,
    issued_at_ms: u64,
    device_token: Option<String>,
) -> ControlPlaneConnectRequest {
    let public_key = signing_key.verifying_key();
    let device_id = "device-1".to_owned();
    let client_id = "cli".to_owned();
    let scopes = BTreeSet::from([ControlPlaneScope::OperatorRead]);
    let signed_at_ms = issued_at_ms.saturating_add(1);

    let mut request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: client_id,
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("LoongClaw CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes,
        caps: BTreeSet::new(),
        commands: BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(loongclaw_protocol::ControlPlaneAuthClaims {
            token: None,
            device_token,
            bootstrap_token: None,
            password: None,
        }),
        device: Some(ControlPlaneDeviceIdentity {
            device_id,
            public_key: base64::engine::general_purpose::STANDARD
                .encode(public_key.as_bytes()),
            signature: String::new(),
            signed_at_ms,
            nonce: nonce.to_owned(),
        }),
    };

    let message = pairing_complete_signature_message(
        &request,
        request.device.as_ref().expect("device"),
    );
    let signature = signing_key.sign(&message);
    request
        .device
        .as_mut()
        .expect("device")
        .signature = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
    request
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

#[tokio::test]
async fn gateway_pairing_start_rejects_missing_auth() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-start-auth");
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/start")
                .method("POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_start_returns_bootstrap_bundle() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-start");
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router(
        "test-token".to_owned(),
        config,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/start")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["protocol"], CONTROL_PLANE_PROTOCOL_VERSION);
    assert_eq!(body["connect_path"], "/v1/pairing/complete");
    assert_eq!(body["pairing_requests_path"], "/v1/pairing/requests");
    assert_eq!(body["pairing_resolve_path"], "/v1/pairing/resolve");
    assert_eq!(body["recommended_role"], "operator");
    assert!(body["challenge"]["nonce"].as_str().is_some());
    assert!(body["challenge"]["expires_at_ms"].as_u64().unwrap_or(0)
        >= body["challenge"]["issued_at_ms"].as_u64().unwrap_or(0));

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_complete_requires_pairing_then_authorizes_after_resolution() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-complete");
    let event_bus = loongclaw_daemon::gateway::event_bus::GatewayEventBus::new(64);
    let app = loongclaw_daemon::gateway::control::build_gateway_pairing_test_router_with_event_bus(
        "test-token".to_owned(),
        config,
        event_bus.clone(),
    );

    let start_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/start")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::OK);
    let start_body = json_body(start_response).await;
    let nonce = start_body["challenge"]["nonce"]
        .as_str()
        .expect("challenge nonce");
    let issued_at_ms = start_body["challenge"]["issued_at_ms"]
        .as_u64()
        .expect("challenge issued_at_ms");
    let signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());

    let initial_request =
        build_signed_pairing_complete_request(&signing_key, nonce, issued_at_ms, None);
    let initial_complete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/complete")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&initial_request).expect("encode pairing complete request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial_complete_response.status(), StatusCode::FORBIDDEN);
    let initial_complete_body = to_bytes(initial_complete_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let initial_error: loongclaw_protocol::ControlPlaneConnectErrorResponse =
        serde_json::from_slice(&initial_complete_body).expect("pairing complete error json");
    assert_eq!(
        initial_error.code,
        loongclaw_protocol::ControlPlaneConnectErrorCode::PairingRequired
    );
    let pairing_request_id = initial_error
        .pairing_request_id
        .expect("pairing request id");

    let resolve_response = app
        .clone()
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
                    .expect("encode resolve request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolve_body = to_bytes(resolve_response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let resolved: ControlPlanePairingResolveResponse =
        serde_json::from_slice(&resolve_body).expect("resolve json");
    let device_token = resolved.device_token.expect("device token");

    let second_start_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/start")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second_start_body = json_body(second_start_response).await;
    let second_nonce = second_start_body["challenge"]["nonce"]
        .as_str()
        .expect("second challenge nonce");
    let second_issued_at_ms = second_start_body["challenge"]["issued_at_ms"]
        .as_u64()
        .expect("second challenge issued_at_ms");

    let authorized_request = build_signed_pairing_complete_request(
        &signing_key,
        second_nonce,
        second_issued_at_ms,
        Some(device_token),
    );
    let authorized_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/complete")
                .method("POST")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&authorized_request)
                        .expect("encode authorized pairing complete request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized_response.status(), StatusCode::OK);
    let authorized_body = json_body(authorized_response).await;
    assert_eq!(authorized_body["status"], "authorized");
    assert_eq!(authorized_body["device_id"], "device-1");
    assert_eq!(authorized_body["role"], "operator");
    let session_token = authorized_body["lease"]["connection_token"]
        .as_str()
        .expect("lease token")
        .to_owned();
    assert!(
        authorized_body["lease"]["connection_token_expires_at_ms"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
    assert_eq!(
        authorized_body["lease"]["principal"]["device_id"],
        "device-1"
    );

    let session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/session")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session_response.status(), StatusCode::OK);
    let session_body = json_body(session_response).await;
    assert_eq!(session_body["status"], "active");
    assert_eq!(session_body["principal"]["device_id"], "device-1");
    assert_eq!(session_body["principal"]["role"], "operator");
    assert_eq!(session_body["last_acknowledged_seq"], serde_json::Value::Null);
    assert_eq!(session_body["resume_status"], "fresh");
    assert_eq!(session_body["resume_from_after_seq"], 0);
    assert_eq!(session_body["earliest_resumable_after_seq"], 0);
    assert_eq!(session_body["replay_window"]["oldest_retained_seq"], serde_json::Value::Null);
    assert_eq!(session_body["replay_window"]["latest_seq"], serde_json::Value::Null);

    event_bus.publish(serde_json::json!({
        "event_type": "pairing.test",
        "message": "hello"
    }));
    event_bus.publish(serde_json::json!({
        "event_type": "pairing.test",
        "message": "world"
    }));

    let pairing_events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/events?after_seq=0&limit=10&ack_seq=2")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pairing_events_response.status(), StatusCode::OK);
    let pairing_events_body = json_body(pairing_events_response).await;
    assert_eq!(pairing_events_body["after_seq"], 0);
    assert_eq!(pairing_events_body["effective_after_seq"], 0);
    assert_eq!(pairing_events_body["returned_count"], 2);
    assert_eq!(pairing_events_body["last_acknowledged_seq"], 2);
    assert_eq!(pairing_events_body["resume_status"], "fresh");
    assert_eq!(pairing_events_body["next_after_seq"], 2);
    assert_eq!(pairing_events_body["earliest_resumable_after_seq"], 0);
    assert_eq!(pairing_events_body["replay_window"]["oldest_retained_seq"], 1);
    assert_eq!(pairing_events_body["replay_window"]["latest_seq"], 2);
    assert_eq!(pairing_events_body["events"][0]["payload"]["message"], "hello");
    assert_eq!(pairing_events_body["events"][1]["payload"]["message"], "world");

    let pairing_stream_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/stream?after_seq=2&limit=10")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pairing_stream_response.status(), StatusCode::OK);
    let pairing_stream_content_type = pairing_stream_response
        .headers()
        .get("content-type")
        .expect("pairing stream content type")
        .to_str()
        .expect("pairing stream content type text");
    assert!(pairing_stream_content_type.starts_with("text/event-stream"));

    let resumed_session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/session")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resumed_session_response.status(), StatusCode::OK);
    let resumed_session_body = json_body(resumed_session_response).await;
    assert_eq!(resumed_session_body["last_acknowledged_seq"], 2);
    assert_eq!(resumed_session_body["resume_status"], "resumed");
    assert_eq!(resumed_session_body["resume_from_after_seq"], 2);
    assert_eq!(resumed_session_body["earliest_resumable_after_seq"], 0);

    let stale_events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/events?after_seq=0&limit=10")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_events_response.status(), StatusCode::OK);

    for index in 0..70 {
        event_bus.publish(serde_json::json!({
            "event_type": "pairing.rollover",
            "index": index,
        }));
    }

    let stale_cursor_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/events?after_seq=1&limit=10")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_cursor_response.status(), StatusCode::CONFLICT);
    let stale_cursor_body = json_body(stale_cursor_response).await;
    assert_eq!(stale_cursor_body["error"]["code"], "stale_cursor");
    assert_eq!(stale_cursor_body["error"]["last_acknowledged_seq"], 2);
    assert!(
        stale_cursor_body["error"]["earliest_resumable_after_seq"]
            .as_u64()
            .unwrap_or(0)
            > 1
    );

    let stale_stream_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/stream?after_seq=1&limit=10")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_stream_response.status(), StatusCode::CONFLICT);
    let stale_stream_body = json_body(stale_stream_response).await;
    assert_eq!(stale_stream_body["error"]["code"], "stale_cursor");

    let stale_session_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/session")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale_session_response.status(), StatusCode::OK);
    let stale_session_body = json_body(stale_session_response).await;
    assert_eq!(stale_session_body["last_acknowledged_seq"], 2);
    assert_eq!(stale_session_body["resume_status"], "stale");
    assert!(
        stale_session_body["resume_from_after_seq"]
            .as_u64()
            .unwrap_or(0)
            > 1
    );

    std::fs::remove_dir_all(root_dir).ok();
}
