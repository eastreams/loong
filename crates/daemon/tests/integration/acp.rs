use super::*;

#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
fn unique_sqlite_path(prefix: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{unique}.sqlite3"))
}

#[test]
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
fn resolve_acp_status_session_key_supports_conversation_lookup() {
    let sqlite_path = unique_sqlite_path("loongclaw-daemon-acp-status");
    let _ = fs::remove_file(&sqlite_path);

    let config = mvp::config::LoongClawConfig {
        memory: mvp::config::MemoryConfig {
            sqlite_path: sqlite_path.display().to_string(),
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongClawConfig::default()
    };
    let store = mvp::acp::AcpSqliteSessionStore::new(Some(sqlite_path));
    mvp::acp::AcpSessionStore::upsert(
        &store,
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
            backend_id: "planning_stub".to_owned(),
            runtime_session_name: "runtime-telegram-42".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-42".to_owned()),
            agent_session_id: Some("agent-42".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Ready,
            last_activity_ms: 1234,
            last_error: None,
        },
    )
    .expect("persist ACP session metadata");

    let resolved = resolve_acp_status_session_key(&config, None, Some("telegram:42"), None)
        .expect("conversation lookup should resolve session key");
    assert_eq!(resolved, "agent:codex:telegram:42");
}

#[test]
#[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
fn resolve_acp_status_session_key_supports_binding_route_lookup() {
    let sqlite_path = unique_sqlite_path("loongclaw-daemon-acp-route-status");
    let _ = fs::remove_file(&sqlite_path);

    let config = mvp::config::LoongClawConfig {
        memory: mvp::config::MemoryConfig {
            sqlite_path: sqlite_path.display().to_string(),
            ..mvp::config::MemoryConfig::default()
        },
        ..mvp::config::LoongClawConfig::default()
    };
    let store = mvp::acp::AcpSqliteSessionStore::new(Some(sqlite_path));
    mvp::acp::AcpSessionStore::upsert(
        &store,
        mvp::acp::AcpSessionMetadata {
            session_key: "agent:codex:opaque-session".to_owned(),
            conversation_id: Some("opaque-session".to_owned()),
            binding: Some(mvp::acp::AcpSessionBindingScope {
                route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                participant_id: None,
                thread_id: Some("om_thread_1".to_owned()),
            }),
            activation_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticDispatch),
            backend_id: "planning_stub".to_owned(),
            runtime_session_name: "runtime-opaque-session".to_owned(),
            working_directory: None,
            backend_session_id: Some("backend-opaque".to_owned()),
            agent_session_id: Some("agent-opaque".to_owned()),
            mode: Some(mvp::acp::AcpSessionMode::Interactive),
            state: mvp::acp::AcpSessionState::Ready,
            last_activity_ms: 4321,
            last_error: None,
        },
    )
    .expect("persist ACP session metadata");

    let resolved = resolve_acp_status_session_key(
        &config,
        None,
        None,
        Some("feishu:lark-prod:oc_123:om_thread_1"),
    )
    .expect("route lookup should resolve session key");
    assert_eq!(resolved, "agent:codex:opaque-session");
}

#[test]
fn acp_session_metadata_json_keeps_activation_provenance_contract() {
    let payload = acp_session_metadata_json(&mvp::acp::AcpSessionMetadata {
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
        backend_id: "planning_stub".to_owned(),
        runtime_session_name: "runtime-telegram-42".to_owned(),
        working_directory: None,
        backend_session_id: Some("backend-42".to_owned()),
        agent_session_id: Some("agent-42".to_owned()),
        mode: Some(mvp::acp::AcpSessionMode::Interactive),
        state: mvp::acp::AcpSessionState::Ready,
        last_activity_ms: 1234,
        last_error: None,
    });

    assert_eq!(payload["activation_origin"], "automatic_dispatch");
    assert_eq!(payload["provenance"]["surface"], "session_activation");
    assert_eq!(
        payload["provenance"]["activation_origin"],
        "automatic_dispatch"
    );
}

#[test]
fn acp_session_status_json_keeps_queue_and_error_fields() {
    let payload = acp_session_status_json(&mvp::acp::AcpSessionStatus {
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
    });

    assert_eq!(payload["session_key"], "agent:codex:telegram:42");
    assert_eq!(payload["conversation_id"], "telegram:42");
    assert_eq!(
        payload["binding"]["route_session_id"],
        "telegram:bot_123456:42"
    );
    assert_eq!(payload["activation_origin"], "automatic_dispatch");
    assert_eq!(payload["provenance"]["surface"], "session_activation");
    assert_eq!(
        payload["provenance"]["activation_origin"],
        "automatic_dispatch"
    );
    assert_eq!(payload["state"], "busy");
    assert_eq!(payload["mode"], "interactive");
    assert_eq!(payload["pending_turns"], 2);
    assert_eq!(payload["active_turn_id"], "runtime-telegram-42");
    assert_eq!(payload["last_activity_ms"], 4567);
    assert_eq!(payload["last_error"], "permission denied");
}

#[test]
fn acp_dispatch_decision_json_keeps_reason_and_structured_target() {
    let payload = acp_dispatch_decision_json(
        "opaque-session",
        &mvp::acp::AcpConversationDispatchDecision {
            route_via_acp: false,
            reason: mvp::acp::AcpConversationDispatchReason::ThreadRequired,
            automatic_routing_origin: None,
            target: mvp::acp::AcpConversationDispatchTarget {
                original_session_id: "opaque-session".to_owned(),
                route_session_id: "feishu:lark-prod:oc_123".to_owned(),
                prefixed_agent_id: None,
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                participant_id: None,
                thread_id: None,
                channel_path: vec!["lark-prod".to_owned(), "oc_123".to_owned()],
            },
        },
    );

    assert_eq!(payload["session"], "opaque-session");
    assert_eq!(payload["decision"]["route_via_acp"], false);
    assert_eq!(payload["decision"]["reason"], "thread_required");
    assert_eq!(
        payload["decision"]["provenance"]["surface"],
        "dispatch_prediction"
    );
    assert_eq!(payload["decision"]["target"]["channel_id"], "feishu");
    assert_eq!(payload["decision"]["target"]["account_id"], "lark-prod");
    assert_eq!(payload["decision"]["target"]["conversation_id"], "oc_123");
    assert_eq!(
        payload["decision"]["target"]["route_session_id"],
        "feishu:lark-prod:oc_123"
    );
    assert_eq!(
        payload["decision"]["automatic_routing_origin"],
        serde_json::Value::Null
    );
    assert_eq!(
        payload["decision"]["provenance"]["automatic_routing_origin"],
        serde_json::Value::Null
    );
}

#[test]
fn acp_dispatch_decision_json_includes_automatic_routing_origin_when_allowed() {
    let payload = acp_dispatch_decision_json(
        "agent:codex:review-thread",
        &mvp::acp::AcpConversationDispatchDecision {
            route_via_acp: true,
            reason: mvp::acp::AcpConversationDispatchReason::Allowed,
            automatic_routing_origin: Some(mvp::acp::AcpRoutingOrigin::AutomaticAgentPrefixed),
            target: mvp::acp::AcpConversationDispatchTarget {
                original_session_id: "agent:codex:review-thread".to_owned(),
                route_session_id: "review-thread".to_owned(),
                prefixed_agent_id: Some("codex".to_owned()),
                channel_id: None,
                account_id: None,
                conversation_id: None,
                participant_id: None,
                thread_id: None,
                channel_path: Vec::new(),
            },
        },
    );

    assert_eq!(payload["decision"]["route_via_acp"], true);
    assert_eq!(payload["decision"]["reason"], "allowed");
    assert_eq!(
        payload["decision"]["automatic_routing_origin"],
        "automatic_agent_prefixed"
    );
    assert_eq!(
        payload["decision"]["provenance"]["surface"],
        "dispatch_prediction"
    );
    assert_eq!(
        payload["decision"]["provenance"]["automatic_routing_origin"],
        "automatic_agent_prefixed"
    );
}

#[test]
fn acp_manager_observability_json_keeps_turn_and_cache_metrics() {
    let payload = acp_manager_observability_json(&mvp::acp::AcpManagerObservabilitySnapshot {
        runtime_cache: mvp::acp::AcpManagerRuntimeCacheSnapshot {
            active_sessions: 3,
            idle_ttl_ms: 60_000,
            evicted_total: 1,
            last_evicted_at_ms: Some(9999),
        },
        sessions: mvp::acp::AcpManagerSessionSnapshot {
            bound: 2,
            unbound: 1,
            activation_origin_counts: std::collections::BTreeMap::from([(
                "automatic_dispatch".to_owned(),
                2usize,
            )]),
            backend_counts: std::collections::BTreeMap::from([(
                "planning_stub".to_owned(),
                3usize,
            )]),
        },
        actors: mvp::acp::AcpManagerActorSnapshot {
            active: 2,
            queue_depth: 3,
            waiting: 1,
        },
        turns: mvp::acp::AcpManagerTurnSnapshot {
            active: 1,
            queue_depth: 2,
            completed: 5,
            failed: 1,
            average_latency_ms: 42,
            max_latency_ms: 88,
        },
        errors_by_code: std::collections::BTreeMap::from([(
            "synthetic failure".to_owned(),
            1usize,
        )]),
    });

    assert_eq!(payload["runtime_cache"]["active_sessions"], 3);
    assert_eq!(payload["runtime_cache"]["idle_ttl_ms"], 60_000);
    assert_eq!(payload["runtime_cache"]["evicted_total"], 1);
    assert_eq!(payload["runtime_cache"]["last_evicted_at_ms"], 9999);
    assert_eq!(payload["sessions"]["bound"], 2);
    assert_eq!(payload["sessions"]["unbound"], 1);
    assert_eq!(
        payload["sessions"]["activation_origin_counts"]["automatic_dispatch"],
        2
    );
    assert_eq!(
        payload["sessions"]["provenance"]["surface"],
        "session_activation_aggregate"
    );
    assert_eq!(
        payload["sessions"]["provenance"]["activation_origin_counts"]["automatic_dispatch"],
        2
    );
    assert_eq!(payload["sessions"]["backend_counts"]["planning_stub"], 3);
    assert_eq!(payload["actors"]["active"], 2);
    assert_eq!(payload["actors"]["queue_depth"], 3);
    assert_eq!(payload["actors"]["waiting"], 1);
    assert_eq!(payload["turns"]["active"], 1);
    assert_eq!(payload["turns"]["queue_depth"], 2);
    assert_eq!(payload["turns"]["completed"], 5);
    assert_eq!(payload["turns"]["failed"], 1);
    assert_eq!(payload["turns"]["average_latency_ms"], 42);
    assert_eq!(payload["turns"]["max_latency_ms"], 88);
    assert_eq!(payload["errors_by_code"]["synthetic failure"], 1);
}

#[test]
fn acp_control_plane_json_keeps_emit_runtime_events_flag() {
    let payload = acp_control_plane_json(&mvp::acp::AcpControlPlaneSnapshot {
        enabled: true,
        dispatch_enabled: false,
        conversation_routing: mvp::config::AcpConversationRoutingMode::AgentPrefixedOnly,
        allowed_channels: vec!["telegram".to_owned(), "feishu".to_owned()],
        allowed_account_ids: vec!["work-bot".to_owned(), "ops-bot".to_owned()],
        bootstrap_mcp_servers: vec!["filesystem".to_owned(), "search".to_owned()],
        working_directory: Some("/workspace/dispatch".to_owned()),
        thread_routing: mvp::config::AcpDispatchThreadRoutingMode::ThreadOnly,
        max_concurrent_sessions: 4,
        session_idle_ttl_ms: 60_000,
        startup_timeout_ms: 10_000,
        turn_timeout_ms: 30_000,
        queue_owner_ttl_ms: 5_000,
        bindings_enabled: true,
        emit_runtime_events: true,
        default_agent: "claude".to_owned(),
        allowed_agents: vec!["codex".to_owned(), "claude".to_owned()],
        allow_mcp_server_injection: false,
    });

    assert_eq!(payload["enabled"], true);
    assert_eq!(payload["dispatch_enabled"], false);
    assert_eq!(payload["conversation_routing"], "agent_prefixed_only");
    assert_eq!(payload["allowed_channels"][0], "telegram");
    assert_eq!(payload["allowed_channels"][1], "feishu");
    assert_eq!(payload["allowed_account_ids"][0], "work-bot");
    assert_eq!(payload["allowed_account_ids"][1], "ops-bot");
    assert_eq!(payload["bootstrap_mcp_servers"][0], "filesystem");
    assert_eq!(payload["bootstrap_mcp_servers"][1], "search");
    assert_eq!(payload["working_directory"], "/workspace/dispatch");
    assert_eq!(payload["thread_routing"], "thread_only");
    assert_eq!(payload["max_concurrent_sessions"], 4);
    assert_eq!(payload["bindings_enabled"], true);
    assert_eq!(payload["emit_runtime_events"], true);
    assert_eq!(payload["default_agent"], "claude");
    assert_eq!(payload["allowed_agents"][0], "codex");
    assert_eq!(payload["allowed_agents"][1], "claude");
    assert_eq!(payload["allow_mcp_server_injection"], false);
}

#[test]
fn acp_event_summary_json_keeps_counts_and_last_fields() {
    let payload = acp_event_summary_json(
        "telegram:42",
        120,
        &mvp::acp::AcpTurnEventSummary {
            turn_event_records: 4,
            final_records: 2,
            done_events: 2,
            error_events: 1,
            text_events: 1,
            usage_update_events: 1,
            turns_succeeded: 1,
            turns_cancelled: 1,
            turns_failed: 0,
            event_type_counts: std::collections::BTreeMap::from([
                ("done".to_owned(), 2u32),
                ("text".to_owned(), 1u32),
            ]),
            stop_reason_counts: std::collections::BTreeMap::from([
                ("completed".to_owned(), 1u32),
                ("cancelled".to_owned(), 1u32),
            ]),
            routing_intent_counts: std::collections::BTreeMap::from([(
                "explicit".to_owned(),
                2u32,
            )]),
            routing_origin_counts: std::collections::BTreeMap::from([(
                "explicit_request".to_owned(),
                2u32,
            )]),
            last_backend_id: Some("acpx".to_owned()),
            last_agent_id: Some("codex".to_owned()),
            last_session_key: Some("agent:codex:telegram:42".to_owned()),
            last_conversation_id: Some("telegram:42".to_owned()),
            last_binding_route_session_id: Some("telegram:bot_123456:42".to_owned()),
            last_channel_id: Some("telegram".to_owned()),
            last_account_id: Some("bot_123456".to_owned()),
            last_channel_conversation_id: Some("42".to_owned()),
            last_channel_participant_id: None,
            last_channel_thread_id: None,
            last_routing_intent: Some("explicit".to_owned()),
            last_routing_origin: Some("explicit_request".to_owned()),
            last_trace_id: Some("trace-123".to_owned()),
            last_source_message_id: Some("message-42".to_owned()),
            last_ack_cursor: Some("cursor-9".to_owned()),
            last_turn_state: Some("ready".to_owned()),
            last_stop_reason: Some("cancelled".to_owned()),
            last_error: Some("permission denied".to_owned()),
        },
    );

    assert_eq!(payload["session"], "telegram:42");
    assert_eq!(payload["limit"], 120);
    assert_eq!(payload["provenance"]["surface"], "turn_execution");
    assert_eq!(payload["provenance"]["last_routing_intent"], "explicit");
    assert_eq!(
        payload["provenance"]["last_routing_origin"],
        "explicit_request"
    );
    assert_eq!(
        payload["provenance"]["routing_intent_counts"]["explicit"],
        2
    );
    assert_eq!(
        payload["provenance"]["routing_origin_counts"]["explicit_request"],
        2
    );
    assert_eq!(payload["summary"]["turn_event_records"], 4);
    assert_eq!(payload["summary"]["final_records"], 2);
    assert_eq!(payload["summary"]["done_events"], 2);
    assert_eq!(payload["summary"]["turns_cancelled"], 1);
    assert_eq!(payload["summary"]["event_type_counts"]["done"], 2);
    assert_eq!(payload["summary"]["stop_reason_counts"]["cancelled"], 1);
    assert_eq!(payload["summary"]["routing_intent_counts"]["explicit"], 2);
    assert_eq!(
        payload["summary"]["routing_origin_counts"]["explicit_request"],
        2
    );
    assert_eq!(payload["summary"]["last_backend_id"], "acpx");
    assert_eq!(payload["summary"]["last_agent_id"], "codex");
    assert_eq!(
        payload["summary"]["last_session_key"],
        "agent:codex:telegram:42"
    );
    assert_eq!(payload["summary"]["last_conversation_id"], "telegram:42");
    assert_eq!(
        payload["summary"]["last_binding_route_session_id"],
        "telegram:bot_123456:42"
    );
    assert_eq!(payload["summary"]["last_routing_intent"], "explicit");
    assert_eq!(
        payload["summary"]["last_routing_origin"],
        "explicit_request"
    );
    assert_eq!(payload["summary"]["last_channel_id"], "telegram");
    assert_eq!(payload["summary"]["last_account_id"], "bot_123456");
    assert_eq!(payload["summary"]["last_channel_conversation_id"], "42");
    assert_eq!(payload["summary"]["last_trace_id"], "trace-123");
    assert_eq!(payload["summary"]["last_source_message_id"], "message-42");
    assert_eq!(payload["summary"]["last_ack_cursor"], "cursor-9");
    assert_eq!(payload["summary"]["last_turn_state"], "ready");
    assert_eq!(payload["summary"]["last_stop_reason"], "cancelled");
    assert_eq!(payload["summary"]["last_error"], "permission denied");
}

#[test]
fn acp_doctor_json_uses_effective_backend_when_cli_overrides_default() {
    let payload = acp_doctor_json(
        "/tmp/loongclaw.toml",
        "planning_stub",
        "acpx",
        &mvp::acp::AcpDoctorReport {
            healthy: true,
            diagnostics: std::collections::BTreeMap::from([(
                "backend".to_owned(),
                "acpx".to_owned(),
            )]),
        },
    );

    assert_eq!(payload["selected_backend"], "acpx");
    assert_eq!(payload["requested_backend"], "acpx");
    assert_eq!(payload["diagnostics"]["backend"], "acpx");
}

#[test]
fn resolve_acp_status_session_key_rejects_missing_selector() {
    let error =
        resolve_acp_status_session_key(&mvp::config::LoongClawConfig::default(), None, None, None)
            .expect_err("missing selector should fail");

    assert!(error.contains("--route-session-id <route_session_id>"));
}
