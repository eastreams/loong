use super::*;
use std::process::Command;

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn gateway_owner_status_fixture() -> loongclaw_daemon::gateway::state::GatewayOwnerStatus {
    loongclaw_daemon::gateway::state::GatewayOwnerStatus {
        runtime_dir: "/tmp/loongclaw-runtime".to_owned(),
        phase: "running".to_owned(),
        running: true,
        stale: false,
        pid: Some(12345),
        mode: loongclaw_daemon::gateway::state::GatewayOwnerMode::GatewayHeadless,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        config_path: "/tmp/loongclaw.toml".to_owned(),
        attached_cli_session: None,
        started_at_ms: 1,
        last_heartbeat_at: 1,
        stopped_at_ms: None,
        shutdown_reason: None,
        last_error: None,
        configured_surface_count: 1,
        running_surface_count: 0,
        bind_address: Some("127.0.0.1".to_owned()),
        port: Some(31337),
        token_path: Some("/tmp/loongclaw.token".to_owned()),
    }
}

fn runtime_snapshot_fixture(
    inventory: &loongclaw_daemon::ChannelsCliJsonPayload,
) -> loongclaw_daemon::gateway::read_models::GatewayRuntimeSnapshotReadModel {
    loongclaw_daemon::gateway::read_models::GatewayRuntimeSnapshotReadModel {
        config: "/tmp/loongclaw.toml".to_owned(),
        schema: loongclaw_daemon::gateway::read_models::GatewayRuntimeSnapshotSchema {
            version: 1,
            surface: "runtime_snapshot",
            purpose: "test",
        },
        provider: serde_json::json!({}),
        context_engine: serde_json::json!({}),
        memory_system: serde_json::json!({}),
        acp: serde_json::json!({}),
        channels: loongclaw_daemon::gateway::read_models::GatewayRuntimeSnapshotChannelsReadModel {
            enabled_channel_ids: vec!["weixin".to_owned()],
            enabled_service_channel_ids: Vec::new(),
            inventory: inventory.clone(),
        },
        tool_runtime: serde_json::json!({}),
        tools: loongclaw_daemon::gateway::read_models::GatewayRuntimeSnapshotToolsReadModel {
            visible_tool_count: 0,
            visible_tool_names: Vec::new(),
            capability_snapshot_sha256: String::new(),
            capability_snapshot: String::new(),
        },
        runtime_plugins: serde_json::json!({}),
        external_skills: serde_json::json!({}),
    }
}

#[test]
fn managed_bridge_parity_keeps_summary_aligned_across_text_json_and_operator_views() {
    let install_root = unique_temp_dir("managed-bridge-parity-module");
    let mut config = mixed_account_weixin_plugin_bridge_config();

    install_ready_weixin_managed_bridge(install_root.as_path());
    config.external_skills.install_root = Some(install_root.display().to_string());

    let inventory = mvp::channel::channel_inventory(&config);
    let rendered =
        loongclaw_daemon::render_channel_surfaces_text("/tmp/loongclaw.toml", &inventory);
    let channels_payload =
        loongclaw_daemon::build_channels_cli_json_payload("/tmp/loongclaw.toml", &inventory);
    let owner_status = gateway_owner_status_fixture();
    let runtime_snapshot = runtime_snapshot_fixture(&channels_payload);
    let operator_summary =
        loongclaw_daemon::gateway::read_models::build_operator_summary_read_model(
            &owner_status,
            &channels_payload,
            &runtime_snapshot,
        );
    let weixin_channels_surface = channels_payload
        .channel_surfaces
        .iter()
        .find(|surface| surface.surface.catalog.id == "weixin")
        .expect("weixin channels surface");
    let weixin_operator_surface = operator_summary
        .channels
        .surfaces
        .iter()
        .find(|surface| surface.channel_id == "weixin")
        .expect("weixin operator surface");

    assert!(
        rendered.contains(MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY),
        "text rendering should keep the shared mixed-account summary visible: {rendered}"
    );
    assert_eq!(
        weixin_channels_surface
            .plugin_bridge_account_summary
            .as_deref(),
        Some(MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY)
    );
    assert_eq!(
        weixin_operator_surface
            .plugin_bridge_account_summary
            .as_deref(),
        Some(MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY)
    );
}

#[test]
fn managed_bridge_parity_keeps_doctor_json_and_channels_json_account_summary_in_sync() {
    let root = unique_temp_dir("managed-bridge-parity-cli-json");
    let install_root = root.join("managed-skills");
    let config_path = root.join("loongclaw.toml");
    let mut config = mixed_account_weixin_plugin_bridge_config();

    install_ready_weixin_managed_bridge(install_root.as_path());
    config.external_skills.install_root = Some(install_root.display().to_string());
    mvp::config::write(
        Some(config_path.to_str().expect("utf8 config path")),
        &config,
        true,
    )
    .expect("write config");

    let doctor_output = Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .arg("doctor")
        .arg("--config")
        .arg(&config_path)
        .arg("--json")
        .output()
        .expect("run doctor json");
    let doctor_stdout = render_output(&doctor_output.stdout);
    let doctor_stderr = render_output(&doctor_output.stderr);
    assert!(
        doctor_output.status.success(),
        "doctor json should succeed, stdout={doctor_stdout:?}, stderr={doctor_stderr:?}"
    );
    let doctor_json: serde_json::Value =
        serde_json::from_slice(&doctor_output.stdout).expect("parse doctor json");
    let doctor_check = doctor_json["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|value| value["name"].as_str() == Some("weixin managed bridge discovery"))
        .expect("weixin doctor check");

    let channels_output = Command::new(env!("CARGO_BIN_EXE_loongclaw"))
        .arg("channels")
        .arg("--config")
        .arg(&config_path)
        .arg("--json")
        .output()
        .expect("run channels json");
    let channels_stdout = render_output(&channels_output.stdout);
    let channels_stderr = render_output(&channels_output.stderr);
    assert!(
        channels_output.status.success(),
        "channels json should succeed, stdout={channels_stdout:?}, stderr={channels_stderr:?}"
    );
    let channels_json: serde_json::Value =
        serde_json::from_slice(&channels_output.stdout).expect("parse channels json");
    let weixin_surface = channels_json["channel_surfaces"]
        .as_array()
        .expect("channel surfaces array")
        .iter()
        .find(|value| value["catalog"]["id"].as_str() == Some("weixin"))
        .expect("weixin channels surface");

    assert_eq!(
        doctor_check["plugin_bridge_account_summary"]
            .as_str()
            .expect("doctor plugin bridge account summary"),
        MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY
    );
    assert_eq!(
        weixin_surface["plugin_bridge_account_summary"]
            .as_str()
            .expect("channels plugin bridge account summary"),
        MIXED_ACCOUNT_WEIXIN_PLUGIN_BRIDGE_SUMMARY
    );
}
