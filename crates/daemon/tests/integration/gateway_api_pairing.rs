use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use loong_daemon::gateway::event_bus::GatewayEventBus;
use loong_daemon::mvp::config::LoongConfig;
use loong_protocol::{
    ControlPlaneAuthClaims, ControlPlaneChallengeResponse, ControlPlaneClientIdentity,
    ControlPlaneConnectRequest, ControlPlanePairingListResponse, ControlPlanePairingResolveRequest,
    ControlPlanePairingResolveResponse, ControlPlanePairingStatus, ControlPlaneRole,
    ControlPlaneScope,
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

fn seed_pending_pairing_request(config: &LoongConfig, public_key: &str) -> String {
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
        public_key,
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

fn gateway_pairing_signature_message(
    request: &ControlPlaneConnectRequest,
    device_id: &str,
    nonce: &str,
    signed_at_ms: u64,
) -> Vec<u8> {
    let scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "loong-control-plane-connect-v1\nnonce={}\ndevice_id={}\nclient_id={}\nrole={}\nscopes={}\nsigned_at_ms={}",
        nonce,
        device_id,
        request.client.id,
        request.role.as_str(),
        scopes,
        signed_at_ms
    )
    .into_bytes()
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
    let signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let public_key = STANDARD.encode(signing_key.verifying_key().as_bytes());
    let pairing_request_id = seed_pending_pairing_request(&config, public_key.as_str());
    let event_bus = GatewayEventBus::new(2);
    let app = loong_daemon::gateway::control::build_gateway_pairing_test_router_with_event_bus(
        "test-token".to_owned(),
        config,
        event_bus.clone(),
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
    let device_token = resolved.device_token.clone().expect("device token");

    let approved_list_response = app
        .clone()
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

    let start_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/pairing/start")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::OK);
    let start_payload: loong_daemon::gateway::read_models::GatewayPairingStartReadModel =
        decode_json(start_response).await;

    let challenge: ControlPlaneChallengeResponse = start_payload.challenge.clone();
    let signed_at_ms = challenge.issued_at_ms;
    let mut connect_request = ControlPlaneConnectRequest {
        min_protocol: loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli-test".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Gateway pairing test".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: BTreeSet::new(),
        commands: BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(ControlPlaneAuthClaims {
            token: None,
            device_token: Some(device_token),
            bootstrap_token: None,
            password: None,
        }),
        device: None,
    };
    let message = gateway_pairing_signature_message(
        &connect_request,
        "device-1",
        challenge.nonce.as_str(),
        signed_at_ms,
    );
    let signature = signing_key.sign(&message);
    connect_request.device = Some(loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-1".to_owned(),
        public_key: public_key.clone(),
        signature: STANDARD.encode(signature.to_bytes()),
        signed_at_ms,
        nonce: challenge.nonce.clone(),
    });

    let complete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/pairing/complete")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&connect_request).expect("encode connect request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(complete_response.status(), StatusCode::OK);
    let complete_payload: loong_daemon::gateway::read_models::GatewayPairingCompleteReadModel =
        decode_json(complete_response).await;
    let session_token = complete_payload.lease.connection_token.clone();

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
    let session_payload: loong_daemon::gateway::read_models::GatewayPairingSessionReadModel =
        decode_json(session_response).await;
    assert_eq!(session_payload.status, "active");
    assert_eq!(session_payload.resume_status, "fresh");

    event_bus.publish(serde_json::json!({"event_type":"one"}));
    event_bus.publish(serde_json::json!({"event_type":"two"}));
    event_bus.publish(serde_json::json!({"event_type":"three"}));

    let stale_response = app
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
    assert_eq!(stale_response.status(), StatusCode::CONFLICT);
    let stale_json: serde_json::Value = decode_json(stale_response).await;
    assert_eq!(stale_json["error"]["code"], "stale_cursor");

    let events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/events?after_seq=1&limit=10&ack_seq=3")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events_response.status(), StatusCode::OK);
    let events_payload: loong_daemon::gateway::read_models::GatewayPairingEventsReadModel =
        decode_json(events_response).await;
    assert_eq!(events_payload.resume_status, "resumed");
    assert_eq!(events_payload.returned_count, 2);
    assert_eq!(events_payload.last_acknowledged_seq, Some(3));

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
    let resumed_session_payload: loong_daemon::gateway::read_models::GatewayPairingSessionReadModel =
        decode_json(resumed_session_response).await;
    assert_eq!(resumed_session_payload.last_acknowledged_seq, Some(3));
    assert_eq!(resumed_session_payload.resume_status, "resumed");

    let stream_response = app
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/stream?after_seq=1&limit=10")
                .header(AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream_response.status(), StatusCode::OK);
    let content_type = stream_response
        .headers()
        .get("content-type")
        .expect("content type")
        .to_str()
        .expect("content type text");
    assert!(content_type.starts_with("text/event-stream"));

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_complete_rejects_invalid_signature() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-invalid-signature");
    let signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let other_signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let public_key = STANDARD.encode(signing_key.verifying_key().as_bytes());
    let event_bus = GatewayEventBus::new(2);
    let app = loong_daemon::gateway::control::build_gateway_pairing_test_router_with_event_bus(
        "test-token".to_owned(),
        config,
        event_bus,
    );

    let start_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/pairing/start")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::OK);
    let start_payload: loong_daemon::gateway::read_models::GatewayPairingStartReadModel =
        decode_json(start_response).await;

    let challenge = start_payload.challenge;
    let signed_at_ms = challenge.issued_at_ms;
    let mut connect_request = ControlPlaneConnectRequest {
        min_protocol: loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: loong_protocol::CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli-invalid-signature".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("Gateway pairing invalid signature test".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
        caps: BTreeSet::new(),
        commands: BTreeSet::new(),
        permissions: std::collections::BTreeMap::new(),
        auth: Some(ControlPlaneAuthClaims::default()),
        device: None,
    };
    let message = gateway_pairing_signature_message(
        &connect_request,
        "device-invalid-signature",
        challenge.nonce.as_str(),
        signed_at_ms,
    );
    let invalid_signature = other_signing_key.sign(&message);
    connect_request.device = Some(loong_protocol::ControlPlaneDeviceIdentity {
        device_id: "device-invalid-signature".to_owned(),
        public_key,
        signature: STANDARD.encode(invalid_signature.to_bytes()),
        signed_at_ms,
        nonce: challenge.nonce,
    });

    let complete_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/pairing/complete")
                .header(AUTHORIZATION, "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&connect_request).expect("encode connect request"),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(complete_response.status(), StatusCode::UNAUTHORIZED);
    let error_json: serde_json::Value = decode_json(complete_response).await;
    assert_eq!(error_json["code"], "device_signature_invalid");

    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_pairing_session_and_events_reject_invalid_session_token() {
    let (config, root_dir) = gateway_pairing_test_config("gateway-pairing-invalid-token");
    let event_bus = GatewayEventBus::new(2);
    let app = loong_daemon::gateway::control::build_gateway_pairing_test_router_with_event_bus(
        "test-token".to_owned(),
        config,
        event_bus,
    );

    let session_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/session")
                .header(AUTHORIZATION, "Bearer invalid-session-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session_response.status(), StatusCode::UNAUTHORIZED);
    let session_json: serde_json::Value = decode_json(session_response).await;
    assert_eq!(session_json["error"]["code"], "invalid_session_token");

    let events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/pairing/events?after_seq=0&limit=10")
                .header(AUTHORIZATION, "Bearer invalid-session-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events_response.status(), StatusCode::UNAUTHORIZED);
    let events_json: serde_json::Value = decode_json(events_response).await;
    assert_eq!(events_json["error"]["code"], "invalid_session_token");

    std::fs::remove_dir_all(root_dir).ok();
}
