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

    let config = mvp::config::LoongConfig::default();
    let config_path = root.join("loong.toml");
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
    let total_surface_count = inventory.channel_surfaces.len();
    let runtime_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let config_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();

    serde_json::json!({
        "config": config_path,
        "schema": {
            "version": CHANNELS_CLI_JSON_SCHEMA_VERSION,
            "primary_channel_view": "channel_surfaces",
            "catalog_view": "channel_catalog",
            "legacy_channel_views": CHANNELS_CLI_JSON_LEGACY_VIEWS,
        },
        "summary": {
            "total_surface_count": total_surface_count,
            "runtime_backed_surface_count": runtime_backed_surface_count,
            "config_backed_surface_count": config_backed_surface_count,
            "plugin_backed_surface_count": plugin_backed_surface_count,
            "catalog_only_surface_count": catalog_only_surface_count,
        },
        "channels": inventory.channels,
        "catalog_only_channels": inventory.catalog_only_channels,
        "channel_catalog": inventory.channel_catalog,
        "channel_surfaces": inventory.channel_surfaces,
        "channel_access_policies": inventory.channel_access_policies,
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

fn legacy_acp_session_list_payload_json(
    config_path: &str,
    matched_count: usize,
    sessions: &[mvp::acp::AcpSessionMetadata],
) -> Value {
    let returned_count = sessions.len();
    let sessions = sessions
        .iter()
        .map(acp_session_metadata_json)
        .collect::<Vec<_>>();

    serde_json::json!({
        "config": config_path,
        "matched_count": matched_count,
        "returned_count": returned_count,
        "sessions": sessions,
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
            "participant_id": address.participant_id,
            "thread_id": address.thread_id,
        },
        "dispatch": acp_dispatch_decision_json(session_id, decision),
    })
}
#[test]
fn gateway_read_model_channel_inventory_matches_channel_cli_contract() {
    let config = mvp::config::LoongConfig::default();
    let inventory = mvp::channel::channel_inventory(&config);
    let payload =
        gateway::read_models::build_channel_inventory_read_model("/tmp/loong.toml", &inventory);
    let encoded = serde_json::to_value(&payload).expect("serialize channel inventory read model");
    let legacy = legacy_channel_inventory_json("/tmp/loong.toml", &inventory);

    assert_eq!(payload.config, "/tmp/loong.toml");
    assert_eq!(payload.schema.version, CHANNELS_CLI_JSON_SCHEMA_VERSION);
    assert_eq!(payload.schema.primary_channel_view, "channel_surfaces");
    assert_eq!(payload.schema.catalog_view, "channel_catalog");
    assert_eq!(
        payload.schema.legacy_channel_views,
        CHANNELS_CLI_JSON_LEGACY_VIEWS
    );
    assert_eq!(
        payload.summary.total_surface_count,
        inventory.channel_surfaces.len()
    );
    assert_eq!(
        payload.summary.runtime_backed_surface_count,
        inventory
            .channel_surfaces
            .iter()
            .filter(|surface| {
                surface.catalog.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
            })
            .count()
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
            participant_id: None,
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
        "/tmp/loong.toml",
        Some("agent:codex:telegram:42"),
        Some("telegram:42"),
        Some("telegram:bot_123456:42"),
        "agent:codex:telegram:42",
        &status,
    );
    let encoded = serde_json::to_value(&payload).expect("serialize ACP status read model");
    let legacy = legacy_acp_status_payload_json(
        "/tmp/loong.toml",
        Some("agent:codex:telegram:42"),
        Some("telegram:42"),
        Some("telegram:bot_123456:42"),
        "agent:codex:telegram:42",
        &status,
    );

    assert_eq!(payload.config, "/tmp/loong.toml");
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
fn gateway_read_model_acp_session_list_keeps_metadata_and_counts() {
    let sessions = vec![
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:telegram:42".to_owned(),
            conversation_id: Some("telegram:42".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "telegram:bot_123456:42".to_owned(),
                channel_id: Some("telegram".to_owned()),
                account_id: Some("bot_123456".to_owned()),
                conversation_id: Some("42".to_owned()),
                participant_id: None,
                thread_id: None,
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-telegram-42".to_owned(),
            working_directory: Some(PathBuf::from("/tmp/runtime-telegram-42")),
            backend_session_id: Some("backend-42".to_owned()),
            agent_session_id: Some("agent-42".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Ready,
            last_activity_ms: 1234,
            last_error: None,
        },
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:feishu:ops".to_owned(),
            conversation_id: Some("feishu:ops".to_owned()),
            binding: None,
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::ExplicitRequest),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-feishu-ops".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-ops".to_owned()),
            agent_session_id: Some("agent-ops".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Error,
            last_activity_ms: 5678,
            last_error: Some("transport failed".to_owned()),
        },
    ];

    let payload =
        gateway::read_models::build_acp_session_list_read_model("/tmp/loong.toml", 9, &sessions);
    let encoded = serde_json::to_value(&payload).expect("serialize ACP session list read model");
    let legacy = legacy_acp_session_list_payload_json("/tmp/loong.toml", 9, &sessions);

    assert_eq!(payload.config, "/tmp/loong.toml");
    assert_eq!(payload.matched_count, 9);
    assert_eq!(payload.returned_count, sessions.len());
    assert_eq!(encoded, legacy);
    assert_eq!(encoded["sessions"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        encoded["sessions"][0]["provenance"]["surface"],
        "session_activation"
    );
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
        gateway::read_models::build_acp_observability_read_model("/tmp/loong.toml", &snapshot);
    let encoded = serde_json::to_value(&payload).expect("serialize ACP observability read model");
    let legacy = legacy_acp_observability_payload_json("/tmp/loong.toml", &snapshot);

    assert_eq!(payload.config, "/tmp/loong.toml");
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
        Some("ou_sender_1"),
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
            participant_id: None,
            thread_id: Some("om_thread_1".to_owned()),
            channel_path: vec![
                "lark-prod".to_owned(),
                "oc_123".to_owned(),
                "om_thread_1".to_owned(),
            ],
        },
    };

    let payload = gateway::read_models::build_acp_dispatch_read_model(
        "/tmp/loong.toml",
        &address,
        "opaque-session",
        &decision,
    );
    let encoded = serde_json::to_value(&payload).expect("serialize ACP dispatch read model");
    let legacy =
        legacy_acp_dispatch_payload_json("/tmp/loong.toml", &address, "opaque-session", &decision);

    assert_eq!(payload.config, "/tmp/loong.toml");
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
    let root = unique_temp_dir("loong-gateway-runtime-snapshot");
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
        payload.channels.enabled_runtime_backed_channel_ids,
        snapshot.enabled_runtime_backed_channel_ids
    );
    assert_eq!(
        payload.channels.enabled_plugin_backed_channel_ids,
        snapshot.enabled_plugin_backed_channel_ids
    );
    assert_eq!(
        payload.channels.enabled_outbound_only_channel_ids,
        snapshot.enabled_outbound_only_channel_ids
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
    assert_eq!(encoded["tools"]["tool_calling"]["availability"], "ready");
    assert_eq!(
        encoded["tools"]["tool_calling"]["structured_tool_schema_enabled"],
        true
    );
    assert_eq!(
        encoded["context_engine"]["compaction"]["preserve_recent_turns"],
        serde_json::json!(6)
    );
    assert_eq!(
        encoded["context_engine"]["compaction"]["hygiene"]["strategy"],
        serde_json::json!("turn_floor_only")
    );
    assert_eq!(
        encoded["context_engine"]["compaction"]["hygiene"]["diagnostics_surface"],
        serde_json::json!("turn_checkpoint")
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["strategy"],
        serde_json::json!("turn_floor_only")
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["diagnostics_surface"],
        serde_json::json!("turn_checkpoint")
    );
    #[cfg(feature = "memory-sqlite")]
    let expected_evidence_status = serde_json::json!("no_evidence");
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_evidence_status = serde_json::json!("idle");
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["evidence_status"],
        expected_evidence_status
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["sampled_session_read_errors"],
        serde_json::json!(0)
    );
    #[cfg(feature = "memory-sqlite")]
    let expected_diagnostics_coverage_milli = serde_json::json!(0);
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_diagnostics_coverage_milli = serde_json::json!(null);
    let expected_failed_open_rate_milli = serde_json::json!(null);
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["diagnostics_coverage_milli"],
        expected_diagnostics_coverage_milli
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["failed_open_rate_milli"],
        expected_failed_open_rate_milli
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["posture"],
        serde_json::json!("idle")
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["sample_order"],
        serde_json::json!("updated_at_desc")
    );
    #[cfg(feature = "memory-sqlite")]
    let expected_trend_scope = serde_json::json!("recent_sessions_fallback");
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_trend_scope = serde_json::json!("idle");
    let expected_reliability_trend = serde_json::json!("insufficient_history");
    #[cfg(feature = "memory-sqlite")]
    let expected_continuity_health = serde_json::json!("scope_limited");
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_continuity_health = serde_json::json!("idle");
    #[cfg(feature = "memory-sqlite")]
    let expected_continuity_repairability = serde_json::json!("scope_limited");
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_continuity_repairability = serde_json::json!("idle");
    #[cfg(feature = "memory-sqlite")]
    let expected_recovery_posture = serde_json::json!("scope_limited");
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_recovery_posture = serde_json::json!("idle");
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["trend_scope"],
        expected_trend_scope
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["reliability_trend"],
        expected_reliability_trend
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["trend_scope"],
        expected_trend_scope
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["continuity_health"],
        expected_continuity_health
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["continuity_repairability"],
        expected_continuity_repairability
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["metrics"]["recovery_posture"],
        expected_recovery_posture
    );
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["strategy"],
        serde_json::json!("turn_floor_only")
    );
    #[cfg(feature = "memory-sqlite")]
    let expected_recent_window_sampled_session_count = serde_json::json!(1);
    #[cfg(not(feature = "memory-sqlite"))]
    let expected_recent_window_sampled_session_count = serde_json::json!(0);
    assert_eq!(
        encoded["context_engine"]["compaction_hygiene"]["recent_window"]["sampled_session_count"],
        expected_recent_window_sampled_session_count
    );
    assert!(encoded["provider"]["transport_runtime"]["failover_total_events"].is_number());
    assert!(encoded["provider"]["transport_runtime"]["failover_by_reason"].is_object());
    assert!(encoded["provider"]["transport_runtime"]["failover_by_stage"].is_object());
    assert!(encoded["provider"]["transport_runtime"]["failover_by_provider"].is_object());
    assert!(encoded["tools"]["web_access"]["ordinary_network_access_enabled"].is_boolean());
    assert!(encoded["tools"]["web_access"]["query_search_enabled"].is_boolean());
    assert!(encoded["tools"]["web_access"]["query_search_default_provider"].is_string());
    assert!(encoded["tools"]["web_access"]["query_search_credential_ready"].is_boolean());
    assert_eq!(
        encoded["tools"]["web_access"]["separation_note"],
        "web-search provider settings affect only query search mode; ordinary network access stays separately governed"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn gateway_read_model_operator_summary_keeps_owner_control_and_runtime_rollups() {
    let root = unique_temp_dir("loong-gateway-operator-summary");
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
        runtime_dir: "/tmp/loong-gateway-runtime".to_owned(),
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
        port_source: None,
        token_path: Some("/tmp/loong-gateway-runtime/control-token".to_owned()),
    };

    let summary = gateway::read_models::build_operator_summary_read_model(
        &owner_status,
        &inventory,
        &runtime_snapshot,
        gateway::read_models::GatewayOperatorPairingSummaryReadModel {
            pending_request_count: 0,
            approved_device_count: 0,
            last_activity_ms: None,
        },
        gateway::read_models::GatewayOperatorNodesSummaryReadModel {
            paired_device_count: 0,
            managed_bridge_count: 0,
            total_count: 0,
        },
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
        summary.channels.plugin_backed_channel_count,
        inventory
            .channel_catalog
            .iter()
            .filter(|channel| {
                channel.implementation_status
                    == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
            })
            .count()
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
        summary.channels.enabled_runtime_backed_channel_count,
        runtime_snapshot
            .channels
            .enabled_runtime_backed_channel_ids
            .len()
    );
    assert_eq!(
        summary.channels.enabled_plugin_backed_channel_count,
        runtime_snapshot
            .channels
            .enabled_plugin_backed_channel_ids
            .len()
    );
    assert_eq!(
        summary.channels.enabled_outbound_only_channel_count,
        runtime_snapshot
            .channels
            .enabled_outbound_only_channel_ids
            .len()
    );
    assert_eq!(
        summary.channels.surfaces.len(),
        inventory.channel_surfaces.len()
    );
    assert_eq!(summary.nodes.paired_device_count, 0);
    assert_eq!(summary.nodes.managed_bridge_count, 0);
    assert_eq!(summary.nodes.total_count, 0);
    assert_eq!(
        summary.runtime.visible_tool_count,
        runtime_snapshot.tools.visible_tool_count
    );
    assert_eq!(
        summary.runtime.enabled_runtime_backed_channel_ids,
        runtime_snapshot.channels.enabled_runtime_backed_channel_ids
    );
    assert_eq!(
        summary.runtime.enabled_plugin_backed_channel_ids,
        runtime_snapshot.channels.enabled_plugin_backed_channel_ids
    );
    assert_eq!(
        summary.runtime.enabled_outbound_only_channel_ids,
        runtime_snapshot.channels.enabled_outbound_only_channel_ids
    );
    assert_eq!(
        summary.runtime.active_provider_profile_id.as_deref(),
        runtime_snapshot.provider["active_profile_id"].as_str()
    );
    assert_eq!(
        summary.runtime.tool_calling.availability,
        runtime_snapshot.tools.tool_calling.availability
    );
    assert_eq!(
        summary.runtime.web_access,
        runtime_snapshot.tools.web_access
    );
    assert_eq!(
        encoded["runtime"]["web_access"]["separation_note"],
        "web-search provider settings affect only query search mode; ordinary network access stays separately governed"
    );
    assert_eq!(
        encoded["control_surface"]["base_url"],
        "http://127.0.0.1:7777"
    );

    fs::remove_dir_all(&root).ok();
}
