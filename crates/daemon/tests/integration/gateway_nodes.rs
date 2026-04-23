use std::collections::BTreeSet;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use tower::ServiceExt;

use super::*;

fn gateway_nodes_test_config(label: &str) -> (mvp::config::LoongConfig, std::path::PathBuf) {
    let root_dir = unique_temp_dir(label);
    std::fs::create_dir_all(root_dir.as_path()).expect("create gateway nodes test dir");

    let sqlite_path = root_dir.join("memory.sqlite3");
    let sqlite_path_text = sqlite_path.display().to_string();
    let install_root = root_dir.join("managed-bridges");
    let mut config = mixed_account_weixin_plugin_bridge_config();
    config.memory.sqlite_path = sqlite_path_text;
    config.external_skills.install_root = Some(install_root.display().to_string());

    (config, root_dir)
}

fn seed_approved_pairing_device(config: &mvp::config::LoongConfig) {
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let session_store_config = mvp::session::store::SessionStoreConfig::from(&memory_config);
    let registry =
        mvp::control_plane::ControlPlanePairingRegistry::with_memory_config(session_store_config)
            .expect("pairing registry");
    let requested_scopes =
        BTreeSet::from(["operator.read".to_owned(), "operator.pairing".to_owned()]);
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
    let pairing_request_id = match decision {
        mvp::control_plane::ControlPlanePairingConnectDecision::PairingRequired {
            request, ..
        } => request.pairing_request_id,
        other => panic!("expected pending pairing request, got {other:?}"),
    };
    let resolved = registry
        .resolve_request(pairing_request_id.as_str(), true)
        .expect("resolve request")
        .expect("resolved record");
    assert!(resolved.device_token.is_some());
}

async fn decode_json(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("decode json")
}

#[tokio::test]
async fn gateway_nodes_reject_missing_auth() {
    let (config, root_dir) = gateway_nodes_test_config("gateway-nodes-auth");
    install_ready_weixin_managed_bridge(root_dir.join("managed-bridges").as_path());
    let inventory = mvp::channel::channel_inventory(&config);
    let channels_payload =
        loong_daemon::build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let app = loong_daemon::gateway::control::build_gateway_nodes_test_router(
        "test-token".to_owned(),
        config,
        channels_payload,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/nodes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    std::fs::remove_dir_all(root_dir).ok();
}

#[tokio::test]
async fn gateway_nodes_return_paired_devices_and_managed_bridges() {
    let (config, root_dir) = gateway_nodes_test_config("gateway-nodes-inventory");
    install_ready_weixin_managed_bridge(root_dir.join("managed-bridges").as_path());
    seed_approved_pairing_device(&config);
    let inventory = mvp::channel::channel_inventory(&config);
    let channels_payload =
        loong_daemon::build_channels_cli_json_payload("/tmp/loong.toml", &inventory);
    let app = loong_daemon::gateway::control::build_gateway_nodes_test_router(
        "test-token".to_owned(),
        config,
        channels_payload,
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/nodes")
                .header(AUTHORIZATION, "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode_json(response).await;

    assert_eq!(body["summary"]["paired_device_count"], 1);
    assert_eq!(body["summary"]["managed_bridge_count"], 1);
    assert_eq!(body["summary"]["total_count"], 2);

    assert_eq!(body["paired_devices"][0]["node_id"], "device-1");
    assert_eq!(body["paired_devices"][0]["node_kind"], "operator_ui");
    assert_eq!(body["paired_devices"][0]["trust_state"], "paired");

    assert_eq!(
        body["managed_bridges"][0]["node_id"],
        "managed_bridge:weixin"
    );
    assert_eq!(body["managed_bridges"][0]["node_kind"], "managed_bridge");
    assert_eq!(body["managed_bridges"][0]["trust_state"], "ready");
    assert_eq!(body["managed_bridges"][0]["channel_id"], "weixin");
    assert_eq!(
        body["managed_bridges"][0]["implementation_status"],
        "plugin_backed"
    );

    std::fs::remove_dir_all(root_dir).ok();
}
