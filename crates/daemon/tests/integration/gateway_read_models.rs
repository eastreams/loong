use super::*;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();
    temp_dir.join(format!("{prefix}-{nanos}"))
}

fn write_gateway_test_config(root: &std::path::Path) -> PathBuf {
    fs::create_dir_all(root).expect("create gateway test root");

    let mut config = mvp::config::LoongClawConfig::default();
    config.audit.path = root
        .join("audit")
        .join("events.jsonl")
        .display()
        .to_string();
    let config_path = root.join("loongclaw.toml");
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    mvp::config::write(Some(config_path_text), &config, true).expect("write gateway test config");

    config_path
}

fn legacy_channel_inventory_json(
    config_path: &str,
    inventory: &mvp::channel::ChannelInventory,
) -> Value {
    serde_json::json!({
        "config": config_path,
        "schema": {
            "version": CHANNELS_CLI_JSON_SCHEMA_VERSION,
            "primary_channel_view": "channel_surfaces",
            "catalog_view": "channel_catalog",
            "legacy_channel_views": CHANNELS_CLI_JSON_LEGACY_VIEWS,
        },
        "channels": inventory.channels,
        "catalog_only_channels": inventory.catalog_only_channels,
        "channel_catalog": inventory.channel_catalog,
        "channel_surfaces": inventory.channel_surfaces,
    })
}

fn legacy_acp_status_payload_json(
    config_path: &str,
    requested_session: Option<&str>,
    requested_conversation_id: Option<&str>,
    requested_route_session_id: Option<&str>,
    resolved_session_key: &str,
    status: &mvp::acp::AcpSessionStatus,
) -> Value {
    serde_json::json!({
        "config": config_path,
        "requested_session": requested_session,
        "requested_conversation_id": requested_conversation_id,
        "requested_route_session_id": requested_route_session_id,
        "resolved_session_key": resolved_session_key,
        "status": acp_session_status_json(status),
    })
}

fn legacy_acp_observability_payload_json(
    config_path: &str,
    snapshot: &mvp::acp::AcpManagerObservabilitySnapshot,
) -> Value {
    serde_json::json!({
        "config": config_path,
        "snapshot": acp_manager_observability_json(snapshot),
    })
}

fn legacy_acp_dispatch_payload_json(
    config_path: &str,
    address: &mvp::conversation::ConversationSessionAddress,
    session_id: &str,
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    serde_json::json!({
        "config": config_path,
        "address": {
            "session_id": address.session_id,
            "channel_id": address.channel_id,
            "account_id": address.account_id,
            "conversation_id": address.conversation_id,
            "thread_id": address.thread_id,
        },
        "dispatch": acp_dispatch_decision_json(session_id, decision),
    })
}
#[test]
fn gateway_read_model_channel_inventory_matches_channel_cli_contract() {
    let config = mvp::config::LoongClawConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload =
        gateway::read_models::build_channel_inventory_read_model("/tmp/loongclaw.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize channel inventory read model");
    let legacy = legacy_channel_inventory_json("/tmp/loongclaw.toml", &inventory);

    assert_eq!(payload.config, "/tmp/loongclaw.toml");
    assert_eq!(payload.schema.version, CHANNELS_CLI_JSON_SCHEMA_VERSION);
    assert_eq!(payload.schema.primary_channel_view, "channel_surfaces");
    assert_eq!(payload.schema.catalog_view, "channel_catalog");
    assert_eq!(
        payload.schema.legacy_channel_views,
        CHANNELS_CLI_JSON_LEGACY_VIEWS
    );
    assert_eq!(encoded, legacy);
    assert_eq!(
        encoded["channel_surfaces"].as_array().map(Vec::len),
        Some(inventory.channel_surfaces.len())
    );
    assert!(
        encoded["channel_catalog"]
            .as_array()
            .expect("channel catalog array")
            .iter()
            .any(|entry| {
                let id = entry.get("id").and_then(Value::as_str);
                let status = entry.get("implementation_status").and_then(Value::as_str);
                id == Some("telegram") && status == Some("runtime_backed")
            })
    );
}

#[test]
fn gateway_read_model_acp_status_keeps_requested_and_resolved_session_fields() {
    let status = mvp::acp::AcpSessionStatus {
        session_key: "agent:codex:telegram:42".to_owned(),
        backend_id: "planning_stub".to_owned(),
        conversation_id: Some("telegram:42".to_owned()),
        binding: Some(mvp::acp::AcpSessionBindingScope {
            route_session_id: "telegram:bot_123456:42".to_owned(),
            channel_id: Some("telegram".to_owned()),
            account_id: Some("bot_123456".to_owned()),
            conversation_id: Some("42".to_owned()),
            thread_id: None,
        }),
        activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
        state: mvp::acp::AcpSessionState::Busy,
        mode: Some(mvp::acp::AcpSessionMode::Interactive),
        pending_turns: 2,
        active_turn_id: Some("runtime-telegram-42".to_owned()),
        last_activity_ms: 4567,
        last_error: Some("permission denied".to_owned()),
    };

    let payload = gateway::read_models::build_acp_status_read_model(
        "/tmp/loongclaw.toml",
        Some("agent:codex:telegram:42"),
        Some("telegram:42"),
        Some("telegram:bot_123456:42"),
        "agent:codex:telegram:42",
        &status,
    );
    let encoded = serde_json::to_value(&payload).expect("serialize ACP status read model");
    let legacy = legacy_acp_status_payload_json(
        "/tmp/loongclaw.toml",
        Some("agent:codex:telegram:42"),
        Some("telegram:42"),
        Some("telegram:bot_123456:42"),
        "agent:codex:telegram:42",
        &status,
    );

    assert_eq!(payload.config, "/tmp/loongclaw.toml");
    assert_eq!(payload.resolved_session_key, "agent:codex:telegram:42");
    assert_eq!(payload.status.state, "busy");
    assert_eq!(payload.status.mode, Some("interactive"));
    assert_eq!(encoded, legacy);
    assert_eq!(
        encoded["status"]["provenance"]["surface"],
        "session_activation"
    );
    assert_eq!(
        encoded["status"]["binding"]["route_session_id"],
        "telegram:bot_123456:42"
    );
    assert_eq!(encoded["status"]["last_error"], "permission denied");
}

#[test]
fn gateway_read_model_acp_observability_keeps_rollups_and_provenance() {
    let mut activation_origin_counts = BTreeMap::new();
    activation_origin_counts.insert("automatic_dispatch".to_owned(), 3);

    let mut backend_counts = BTreeMap::new();
    backend_counts.insert("planning_stub".to_owned(), 2);

    let mut errors_by_code = BTreeMap::new();
    errors_by_code.insert("timeout".to_owned(), 1);

    let snapshot = mvp::acp::AcpManagerObservabilitySnapshot {
        runtime_cache: mvp::acp::AcpManagerRuntimeCacheSnapshot {
            active_sessions: 2,
            idle_ttl_ms: 30_000,
            evicted_total: 4,
            last_evicted_at_ms: Some(1234),
        },
        sessions: mvp::acp::AcpManagerSessionSnapshot {
            bound: 1,
            unbound: 1,
            activation_origin_counts,
            backend_counts,
        },
        actors: mvp::acp::AcpManagerActorSnapshot {
            active: 1,
            queue_depth: 2,
            waiting: 3,
        },
        turns: mvp::acp::AcpManagerTurnSnapshot {
            active: 1,
            queue_depth: 2,
            completed: 8,
            failed: 1,
            average_latency_ms: 42,
            max_latency_ms: 99,
        },
        errors_by_code,
    };

    let payload =
        gateway::read_models::build_acp_observability_read_model("/tmp/loongclaw.toml", &snapshot);
    let encoded = serde_json::to_value(&payload).expect("serialize ACP observability read model");
    let legacy = legacy_acp_observability_payload_json("/tmp/loongclaw.toml", &snapshot);

    assert_eq!(payload.config, "/tmp/loongclaw.toml");
    assert_eq!(payload.snapshot.runtime_cache.active_sessions, 2);
    assert_eq!(payload.snapshot.sessions.bound, 1);
    assert_eq!(payload.snapshot.turns.completed, 8);
    assert_eq!(encoded, legacy);
    assert_eq!(
        encoded["snapshot"]["sessions"]["provenance"]["surface"],
        "session_activation_aggregate"
    );
    assert_eq!(encoded["snapshot"]["errors_by_code"]["timeout"], 1);
}

#[test]
fn gateway_read_model_acp_dispatch_keeps_structured_address_and_target() {
    let address = build_acp_dispatch_address(
        "opaque-session",
        Some("feishu"),
        Some("oc_123"),
        Some("lark-prod"),
        Some("om_thread_1"),
    )
    .expect("build ACP dispatch address");
    let decision = mvp::acp::AcpConversationDispatchDecision {
        route_via_acp: true,
        reason: mvp::acp::AcpConversationDispatchReason::Allowed,
        automatic_routing_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticAgentPrefixed),
        target: mvp::acp::AcpConversationDispatchTarget {
            original_session_id: "opaque-session".to_owned(),
            route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
            prefixed_agent_id: Some("codex".to_owned()),
            channel_id: Some("feishu".to_owned()),
            account_id: Some("lark-prod".to_owned()),
            conversation_id: Some("oc_123".to_owned()),
            thread_id: Some("om_thread_1".to_owned()),
            channel_path: vec![
                "lark-prod".to_owned(),
                "oc_123".to_owned(),
                "om_thread_1".to_owned(),
            ],
        },
    };

    let payload = gateway::read_models::build_acp_dispatch_read_model(
        "/tmp/loongclaw.toml",
        &address,
        "opaque-session",
        &decision,
    );
    let encoded = serde_json::to_value(&payload).expect("serialize ACP dispatch read model");
    let legacy = legacy_acp_dispatch_payload_json(
        "/tmp/loongclaw.toml",
        &address,
        "opaque-session",
        &decision,
    );

    assert_eq!(payload.config, "/tmp/loongclaw.toml");
    assert_eq!(payload.address.channel_id.as_deref(), Some("feishu"));
    assert_eq!(payload.dispatch.session, "opaque-session");
    assert_eq!(payload.dispatch.decision.reason, "allowed");
    assert_eq!(encoded, legacy);
    assert_eq!(
        encoded["dispatch"]["decision"]["provenance"]["surface"],
        "dispatch_prediction"
    );
    assert_eq!(
        encoded["dispatch"]["decision"]["target"]["route_session_id"],
        "feishu:lark-prod:oc_123:om_thread_1"
    );
}

#[test]
fn gateway_read_model_runtime_snapshot_embeds_inventory_and_tool_summary() {
    let root = unique_temp_dir("loongclaw-gateway-runtime-snapshot");
    let config_path = write_gateway_test_config(&root);
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    let snapshot = collect_runtime_snapshot_cli_state(Some(config_path_text))
        .expect("collect runtime snapshot");
    let payload = gateway::read_models::build_runtime_snapshot_read_model(&snapshot);
    let encoded = serde_json::to_value(&payload).expect("serialize runtime snapshot read model");

    assert_eq!(
        payload.schema.version,
        RUNTIME_SNAPSHOT_CLI_JSON_SCHEMA_VERSION
    );
    assert_eq!(payload.schema.surface, "runtime_snapshot");
    assert_eq!(
        payload.channels.inventory.schema.primary_channel_view,
        "channel_surfaces"
    );
    assert_eq!(
        payload.tools.visible_tool_count,
        payload.tools.visible_tool_names.len()
    );
    assert_eq!(
        encoded["channels"]["inventory"]["schema"]["catalog_view"],
        "channel_catalog"
    );
    assert!(
        encoded["tools"]["visible_tool_count"]
            .as_u64()
            .is_some_and(|value| value > 0),
        "runtime snapshot should advertise at least one visible tool"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn gateway_read_model_operator_summary_keeps_owner_control_and_runtime_rollups() {
    let root = unique_temp_dir("loongclaw-gateway-operator-summary");
    let config_path = write_gateway_test_config(&root);
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");

    let snapshot = collect_runtime_snapshot_cli_state(Some(config_path_text))
        .expect("collect runtime snapshot");
    let inventory = gateway::read_models::build_channel_inventory_read_model(
        config_path_text,
        &snapshot.channels,
    );
    let runtime_snapshot = gateway::read_models::build_runtime_snapshot_read_model(&snapshot);
    let owner_status = gateway::state::GatewayOwnerStatus {
        runtime_dir: "/tmp/loongclaw-gateway-runtime".to_owned(),
        phase: "running".to_owned(),
        running: true,
        stale: false,
        pid: Some(42),
        mode: gateway::state::GatewayOwnerMode::GatewayHeadless,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        config_path: config_path_text.to_owned(),
        attached_cli_session: None,
        started_at_ms: 100,
        last_heartbeat_at: 200,
        stopped_at_ms: None,
        shutdown_reason: None,
        last_error: None,
        configured_surface_count: 0,
        running_surface_count: 0,
        bind_address: Some("127.0.0.1".to_owned()),
        port: Some(7777),
        token_path: Some("/tmp/loongclaw-gateway-runtime/control-token".to_owned()),
    };

    let summary = gateway::read_models::build_operator_summary_read_model(
        &owner_status,
        &inventory,
        &runtime_snapshot,
    );
    let encoded = serde_json::to_value(&summary).expect("serialize operator summary read model");

    assert_eq!(summary.owner.phase, "running");
    assert_eq!(
        summary.control_surface.base_url.as_deref(),
        Some("http://127.0.0.1:7777")
    );
    assert!(summary.control_surface.loopback_only);
    assert_eq!(
        summary.channels.catalog_channel_count,
        inventory.channel_catalog.len()
    );
    assert_eq!(
        summary.channels.configured_account_count,
        inventory.channels.len()
    );
    assert_eq!(
        summary.channels.enabled_service_channel_count,
        runtime_snapshot.channels.enabled_service_channel_ids.len()
    );
    assert_eq!(
        summary.channels.surfaces.len(),
        inventory.channel_surfaces.len()
    );
    assert_eq!(
        summary.runtime.visible_tool_count,
        runtime_snapshot.tools.visible_tool_count
    );
    assert_eq!(
        summary.runtime.active_provider_profile_id.as_deref(),
        runtime_snapshot.provider["active_profile_id"].as_str()
    );
    assert_eq!(
        encoded["control_surface"]["base_url"],
        "http://127.0.0.1:7777"
    );

    fs::remove_dir_all(&root).ok();
}
