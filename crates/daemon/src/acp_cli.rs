use serde_json::{Value, json};

use crate::{
    CliResult, format_capability_names, format_u32_rollup, format_usize_rollup, gateway, mvp,
};

pub fn run_list_acp_backends_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::acp::collect_acp_runtime_snapshot(&config)?;

    if as_json {
        let payload = json!({
            "config": resolved_path.display().to_string(),
            "enabled": snapshot.control_plane.enabled,
            "selected": acp_backend_metadata_json(
                &snapshot.selected_metadata,
                Some(snapshot.selected.source.as_str())
            ),
            "available": snapshot
                .available
                .iter()
                .map(|metadata| acp_backend_metadata_json(metadata, None))
                .collect::<Vec<_>>(),
            "control_plane": acp_control_plane_json(&snapshot.control_plane),
        });
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP backend output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "enabled={} selected={} source={} api_version={} capabilities={}",
        snapshot.control_plane.enabled,
        snapshot.selected_metadata.id,
        snapshot.selected.source.as_str(),
        snapshot.selected_metadata.api_version,
        format_capability_names(&snapshot.selected_metadata.capability_names())
    );
    println!(
        "control_plane=dispatch_enabled:{} conversation_routing:{} allowed_channels:{} allowed_account_ids:{} bootstrap_mcp_servers:{} working_directory:{} thread_routing:{} default_agent:{} allowed_agents:{} max_concurrent_sessions:{} session_idle_ttl_ms:{} startup_timeout_ms:{} turn_timeout_ms:{} queue_owner_ttl_ms:{} bindings_enabled:{} emit_runtime_events:{} allow_mcp_server_injection:{}",
        snapshot.control_plane.dispatch_enabled,
        snapshot.control_plane.conversation_routing.as_str(),
        snapshot.control_plane.allowed_channels.join(","),
        snapshot.control_plane.allowed_account_ids.join(","),
        snapshot.control_plane.bootstrap_mcp_servers.join(","),
        snapshot
            .control_plane
            .working_directory
            .as_deref()
            .unwrap_or(""),
        snapshot.control_plane.thread_routing.as_str(),
        snapshot.control_plane.default_agent,
        snapshot.control_plane.allowed_agents.join(","),
        snapshot.control_plane.max_concurrent_sessions,
        snapshot.control_plane.session_idle_ttl_ms,
        snapshot.control_plane.startup_timeout_ms,
        snapshot.control_plane.turn_timeout_ms,
        snapshot.control_plane.queue_owner_ttl_ms,
        snapshot.control_plane.bindings_enabled,
        snapshot.control_plane.emit_runtime_events,
        snapshot.control_plane.allow_mcp_server_injection
    );
    println!("available:");
    for metadata in snapshot.available {
        println!(
            "- {} api_version={} capabilities={} summary={}",
            metadata.id,
            metadata.api_version,
            format_capability_names(&metadata.capability_names()),
            metadata.summary
        );
    }
    Ok(())
}

pub fn run_list_acp_sessions_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
    {
        let _ = (config_path, as_json);
        Err("ACP session persistence requires feature `memory-sqlite`".to_owned())
    }

    #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
    {
        let (resolved_path, config) = mvp::config::load(config_path)?;
        let store =
            mvp::acp::AcpSqliteSessionStore::new(Some(config.memory.resolved_sqlite_path()));
        let sessions = mvp::acp::AcpSessionStore::list(&store)?;

        if as_json {
            let payload = json!({
                "config": resolved_path.display().to_string(),
                "sqlite_path": config.memory.resolved_sqlite_path().display().to_string(),
                "sessions": sessions
                    .iter()
                    .map(acp_session_metadata_json)
                    .collect::<Vec<_>>(),
            });
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize ACP session output failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }

        println!(
            "config={} sqlite_path={}",
            resolved_path.display(),
            config.memory.resolved_sqlite_path().display()
        );
        if sessions.is_empty() {
            println!("sessions: (none)");
            return Ok(());
        }
        println!("sessions:");
        for session in sessions {
            println!(
                "- session_key={} backend={} conversation_id={} binding_route_session_id={} activation_origin={} state={} mode={} runtime_session_name={} last_activity_ms={} last_error={}",
                session.session_key,
                session.backend_id,
                session.conversation_id.as_deref().unwrap_or("(none)"),
                session
                    .binding
                    .as_ref()
                    .map(|binding| binding.route_session_id.as_str())
                    .unwrap_or("(none)"),
                session
                    .activation_origin
                    .map(mvp::acp::AcpRoutingOrigin::as_str)
                    .unwrap_or("(none)"),
                acp_session_state_label(session.state),
                session.mode.map(acp_session_mode_label).unwrap_or("(none)"),
                session.runtime_session_name,
                session.last_activity_ms,
                session.last_error.as_deref().unwrap_or("(none)")
            );
        }
        Ok(())
    }
}

pub async fn run_acp_doctor_cli(
    config_path: Option<&str>,
    backend_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let selection = mvp::acp::resolve_acp_backend_selection(&config);
    let backend = backend_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(selection.id.as_str());
    let report = mvp::acp::AcpSessionManager::default()
        .doctor(&config, Some(backend))
        .await?;

    if as_json {
        let payload = acp_doctor_json(
            resolved_path.display().to_string(),
            selection.id.as_str(),
            backend,
            &report,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP doctor output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "selected_backend={} requested_backend={} healthy={}",
        backend, backend, report.healthy
    );
    if report.diagnostics.is_empty() {
        println!("diagnostics: (none)");
        return Ok(());
    }
    println!("diagnostics:");
    for (key, value) in report.diagnostics {
        println!("- {}={}", key, value);
    }
    Ok(())
}

pub fn acp_doctor_json(
    config_path: impl Into<String>,
    _default_backend: &str,
    effective_backend: &str,
    report: &mvp::acp::AcpDoctorReport,
) -> Value {
    json!({
        "config": config_path.into(),
        "selected_backend": effective_backend,
        "requested_backend": effective_backend,
        "healthy": report.healthy,
        "diagnostics": report.diagnostics,
    })
}

pub async fn run_acp_status_cli(
    config_path: Option<&str>,
    session_key: Option<&str>,
    conversation_id: Option<&str>,
    route_session_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let resolved_session_key =
        resolve_acp_status_session_key(&config, session_key, conversation_id, route_session_id)?;
    let manager = mvp::acp::shared_acp_session_manager(&config)?;
    let status = manager
        .get_status(&config, resolved_session_key.as_str())
        .await?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_status_read_model(
            config_display.as_str(),
            session_key,
            conversation_id,
            route_session_id,
            resolved_session_key.as_str(),
            &status,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP status output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    if let Some(conversation_id) = conversation_id {
        println!("requested_conversation_id={conversation_id}");
    }
    if let Some(route_session_id) = route_session_id {
        println!("requested_route_session_id={route_session_id}");
    }
    if let Some(session_key) = session_key {
        println!("requested_session={session_key}");
    }
    println!("resolved_session_key={}", resolved_session_key);
    println!(
        "status=backend:{} state:{} mode:{} pending_turns:{} active_turn_id:{} conversation_id:{} binding_route_session_id:{} activation_origin:{} last_activity_ms:{} last_error={}",
        status.backend_id,
        acp_session_state_label(status.state),
        status.mode.map(acp_session_mode_label).unwrap_or("(none)"),
        status.pending_turns,
        status.active_turn_id.as_deref().unwrap_or("(none)"),
        status.conversation_id.as_deref().unwrap_or("(none)"),
        status
            .binding
            .as_ref()
            .map(|binding| binding.route_session_id.as_str())
            .unwrap_or("(none)"),
        status
            .activation_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str)
            .unwrap_or("(none)"),
        status.last_activity_ms,
        status.last_error.as_deref().unwrap_or("(none)")
    );
    Ok(())
}

pub async fn run_acp_observability_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let manager = mvp::acp::shared_acp_session_manager(&config)?;
    let snapshot = manager.observability_snapshot(&config).await?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_observability_read_model(
            config_display.as_str(),
            &snapshot,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP observability output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "runtime_cache=active_sessions:{} idle_ttl_ms:{} evicted_total:{} last_evicted_at_ms:{}",
        snapshot.runtime_cache.active_sessions,
        snapshot.runtime_cache.idle_ttl_ms,
        snapshot.runtime_cache.evicted_total,
        snapshot
            .runtime_cache
            .last_evicted_at_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "(none)".to_owned())
    );
    println!(
        "sessions=bound:{} unbound:{} activation_origins:{} backends:{}",
        snapshot.sessions.bound,
        snapshot.sessions.unbound,
        format_usize_rollup(&snapshot.sessions.activation_origin_counts),
        format_usize_rollup(&snapshot.sessions.backend_counts)
    );
    println!(
        "actors=active:{} queue_depth:{} waiting:{}",
        snapshot.actors.active, snapshot.actors.queue_depth, snapshot.actors.waiting
    );
    println!(
        "turns=active:{} queue_depth:{} completed:{} failed:{} average_latency_ms:{} max_latency_ms:{}",
        snapshot.turns.active,
        snapshot.turns.queue_depth,
        snapshot.turns.completed,
        snapshot.turns.failed,
        snapshot.turns.average_latency_ms,
        snapshot.turns.max_latency_ms
    );
    if snapshot.errors_by_code.is_empty() {
        println!("errors_by_code: (none)");
    } else {
        println!("errors_by_code:");
        for (key, value) in snapshot.errors_by_code {
            println!("- {}={}", key, value);
        }
    }
    Ok(())
}

pub fn resolve_acp_status_session_key(
    config: &mvp::config::LoongClawConfig,
    session_key: Option<&str>,
    conversation_id: Option<&str>,
    route_session_id: Option<&str>,
) -> CliResult<String> {
    let session_key = session_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let route_session_id = route_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    match (session_key, conversation_id, route_session_id) {
        (Some(session_key), None, None) => Ok(session_key),
        (None, Some(conversation_id), None) => {
            #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
            {
                let _ = (config, conversation_id);
                Err("ACP conversation-id lookup requires feature `memory-sqlite`".to_owned())
            }

            #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
            {
                let store = mvp::acp::AcpSqliteSessionStore::new(Some(
                    config.memory.resolved_sqlite_path(),
                ));
                let metadata = mvp::acp::AcpSessionStore::get_by_conversation_id(
                    &store,
                    conversation_id.as_str(),
                )?
                .ok_or_else(|| {
                    format!(
                        "ACP conversation `{}` is not registered in {}",
                        conversation_id,
                        config.memory.resolved_sqlite_path().display()
                    )
                })?;
                Ok(metadata.session_key)
            }
        }
        (None, None, Some(route_session_id)) => {
            #[cfg(not(any(feature = "memory-sqlite", feature = "mvp")))]
            {
                let _ = (config, route_session_id);
                Err("ACP route-session-id lookup requires feature `memory-sqlite`".to_owned())
            }

            #[cfg(any(feature = "memory-sqlite", feature = "mvp"))]
            {
                let store = mvp::acp::AcpSqliteSessionStore::new(Some(
                    config.memory.resolved_sqlite_path(),
                ));
                let metadata = mvp::acp::AcpSessionStore::get_by_binding_route_session_id(
                    &store,
                    route_session_id.as_str(),
                )?
                .ok_or_else(|| {
                    format!(
                        "ACP route session `{}` is not registered in {}",
                        route_session_id,
                        config.memory.resolved_sqlite_path().display()
                    )
                })?;
                Ok(metadata.session_key)
            }
        }
        (Some(_), Some(_), _)
        | (Some(_), _, Some(_))
        | (_, Some(_), Some(_)) => Err(
            "acp-status accepts exactly one of --session, --conversation-id, or --route-session-id"
                .to_owned(),
        ),
        (None, None, None) => Err(
            "acp-status requires --session <session_key>, --conversation-id <conversation_id>, or --route-session-id <route_session_id>"
                .to_owned(),
        ),
    }
}

pub fn run_acp_event_summary_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    limit: usize,
    as_json: bool,
) -> CliResult<()> {
    if limit == 0 {
        return Err("acp-event-summary limit must be >= 1".to_owned());
    }

    let (_, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = mvp::memory::window_direct(&session_id, limit, &mem_config)
            .map_err(|error| format!("load ACP event summary failed: {error}"))?;
        let summary = mvp::acp::summarize_turn_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        if as_json {
            let payload = acp_event_summary_json(&session_id, limit, &summary);
            let pretty = serde_json::to_string_pretty(&payload)
                .map_err(|error| format!("serialize ACP event summary failed: {error}"))?;
            println!("{pretty}");
            return Ok(());
        }
        print!("{}", format_acp_event_summary(&session_id, limit, &summary));
        Ok(())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, session_id, as_json);
        Err("acp-event-summary requires memory-sqlite feature".to_owned())
    }
}

pub fn run_acp_dispatch_cli(
    config_path: Option<&str>,
    session: Option<&str>,
    channel: Option<&str>,
    conversation_id: Option<&str>,
    account_id: Option<&str>,
    thread_id: Option<&str>,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let session_id = session
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_owned();
    let address = build_acp_dispatch_address(
        session_id.as_str(),
        channel,
        conversation_id,
        account_id,
        thread_id,
    )?;
    let decision = mvp::acp::evaluate_acp_conversation_dispatch_for_address(&config, &address)?;

    if as_json {
        let config_display = resolved_path.display().to_string();
        let payload = gateway::read_models::build_acp_dispatch_read_model(
            config_display.as_str(),
            &address,
            session_id.as_str(),
            &decision,
        );
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize ACP dispatch output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!("config={}", resolved_path.display());
    println!(
        "address=session:{} channel:{} account_id:{} conversation_id:{} thread_id:{}",
        address.session_id,
        address.channel_id.as_deref().unwrap_or("(none)"),
        address.account_id.as_deref().unwrap_or("(none)"),
        address.conversation_id.as_deref().unwrap_or("(none)"),
        address.thread_id.as_deref().unwrap_or("(none)")
    );
    println!(
        "dispatch=route_via_acp:{} reason:{} automatic_routing_origin:{} route_session_id:{} prefixed_agent_id:{} channel_id:{} account_id:{} conversation_id:{} thread_id:{}",
        decision.route_via_acp,
        decision.reason.as_str(),
        decision
            .automatic_routing_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str)
            .unwrap_or("(none)"),
        decision.target.route_session_id,
        decision
            .target
            .prefixed_agent_id
            .as_deref()
            .unwrap_or("(none)"),
        decision.target.channel_id.as_deref().unwrap_or("(none)"),
        decision.target.account_id.as_deref().unwrap_or("(none)"),
        decision
            .target
            .conversation_id
            .as_deref()
            .unwrap_or("(none)"),
        decision.target.thread_id.as_deref().unwrap_or("(none)")
    );
    println!(
        "channel_path={}",
        if decision.target.channel_path.is_empty() {
            "(none)".to_owned()
        } else {
            decision.target.channel_path.join(":")
        }
    );
    Ok(())
}

pub fn build_acp_dispatch_address(
    session_id: &str,
    channel: Option<&str>,
    conversation_id: Option<&str>,
    account_id: Option<&str>,
    thread_id: Option<&str>,
) -> CliResult<mvp::conversation::ConversationSessionAddress> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("acp-dispatch requires a non-empty --session value".to_owned());
    }

    let channel = channel.map(str::trim).filter(|value| !value.is_empty());
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account_id = account_id.map(str::trim).filter(|value| !value.is_empty());
    let thread_id = thread_id.map(str::trim).filter(|value| !value.is_empty());

    let channel = match channel {
        Some(channel) => channel,
        None => {
            if conversation_id.is_some() || account_id.is_some() || thread_id.is_some() {
                return Err(
                    "acp-dispatch requires --channel when using --conversation-id, --account-id, or --thread-id"
                        .to_owned(),
                );
            }
            return Ok(mvp::conversation::ConversationSessionAddress::from_session_id(session_id));
        }
    };

    let conversation_id = conversation_id.ok_or_else(|| {
        "acp-dispatch requires --conversation-id when --channel is provided".to_owned()
    })?;
    let mut address = mvp::conversation::ConversationSessionAddress::from_session_id(session_id)
        .with_channel_scope(channel, conversation_id);
    if let Some(account_id) = account_id {
        address = address.with_account_id(account_id);
    }
    if let Some(thread_id) = thread_id {
        address = address.with_thread_id(thread_id);
    }
    Ok(address)
}

pub fn acp_backend_metadata_json(
    metadata: &mvp::acp::AcpBackendMetadata,
    source: Option<&str>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_owned(), json!(metadata.id));
    payload.insert("api_version".to_owned(), json!(metadata.api_version));
    payload.insert(
        "capabilities".to_owned(),
        json!(metadata.capability_names()),
    );
    payload.insert("summary".to_owned(), json!(metadata.summary));
    if let Some(source) = source {
        payload.insert("source".to_owned(), json!(source));
    }
    Value::Object(payload)
}

pub fn acp_control_plane_json(snapshot: &mvp::acp::AcpControlPlaneSnapshot) -> Value {
    json!({
        "enabled": snapshot.enabled,
        "dispatch_enabled": snapshot.dispatch_enabled,
        "conversation_routing": snapshot.conversation_routing.as_str(),
        "allowed_channels": snapshot.allowed_channels,
        "allowed_account_ids": snapshot.allowed_account_ids,
        "bootstrap_mcp_servers": snapshot.bootstrap_mcp_servers,
        "working_directory": snapshot.working_directory,
        "thread_routing": snapshot.thread_routing.as_str(),
        "default_agent": snapshot.default_agent,
        "allowed_agents": snapshot.allowed_agents,
        "max_concurrent_sessions": snapshot.max_concurrent_sessions,
        "session_idle_ttl_ms": snapshot.session_idle_ttl_ms,
        "startup_timeout_ms": snapshot.startup_timeout_ms,
        "turn_timeout_ms": snapshot.turn_timeout_ms,
        "queue_owner_ttl_ms": snapshot.queue_owner_ttl_ms,
        "bindings_enabled": snapshot.bindings_enabled,
        "emit_runtime_events": snapshot.emit_runtime_events,
        "allow_mcp_server_injection": snapshot.allow_mcp_server_injection,
    })
}

pub fn acp_session_metadata_json(metadata: &mvp::acp::AcpSessionMetadata) -> Value {
    json!({
        "session_key": metadata.session_key,
        "conversation_id": metadata.conversation_id,
        "binding": metadata.binding.as_ref().map(acp_binding_scope_json),
        "activation_origin": metadata.activation_origin.map(mvp::acp::AcpRoutingOrigin::as_str),
        "provenance": acp_session_activation_provenance_json(metadata.activation_origin),
        "backend_id": metadata.backend_id,
        "runtime_session_name": metadata.runtime_session_name,
        "working_directory": metadata
            .working_directory
            .as_ref()
            .map(|path| path.display().to_string()),
        "backend_session_id": metadata.backend_session_id,
        "agent_session_id": metadata.agent_session_id,
        "mode": metadata.mode.map(acp_session_mode_label),
        "state": acp_session_state_label(metadata.state),
        "last_activity_ms": metadata.last_activity_ms,
        "last_error": metadata.last_error,
    })
}

pub fn acp_session_status_json(status: &mvp::acp::AcpSessionStatus) -> Value {
    json!({
        "session_key": status.session_key,
        "backend_id": status.backend_id,
        "conversation_id": status.conversation_id,
        "binding": status.binding.as_ref().map(acp_binding_scope_json),
        "activation_origin": status.activation_origin.map(mvp::acp::AcpRoutingOrigin::as_str),
        "provenance": acp_session_activation_provenance_json(status.activation_origin),
        "state": acp_session_state_label(status.state),
        "mode": status.mode.map(acp_session_mode_label),
        "pending_turns": status.pending_turns,
        "active_turn_id": status.active_turn_id,
        "last_activity_ms": status.last_activity_ms,
        "last_error": status.last_error,
    })
}

pub fn acp_binding_scope_json(binding: &mvp::acp::AcpSessionBindingScope) -> Value {
    json!({
        "route_session_id": binding.route_session_id,
        "channel_id": binding.channel_id,
        "account_id": binding.account_id,
        "conversation_id": binding.conversation_id,
        "thread_id": binding.thread_id,
    })
}

pub fn acp_session_activation_provenance_json(origin: Option<mvp::acp::AcpRoutingOrigin>) -> Value {
    json!({
        "surface": "session_activation",
        "activation_origin": origin.map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

pub fn acp_dispatch_prediction_provenance_json(
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    json!({
        "surface": "dispatch_prediction",
        "automatic_routing_origin": decision
            .automatic_routing_origin
            .map(mvp::acp::AcpRoutingOrigin::as_str),
    })
}

pub fn acp_turn_provenance_json(summary: &mvp::acp::AcpTurnEventSummary) -> Value {
    json!({
        "surface": "turn_execution",
        "last_routing_intent": summary.last_routing_intent,
        "last_routing_origin": summary.last_routing_origin,
        "routing_intent_counts": summary.routing_intent_counts,
        "routing_origin_counts": summary.routing_origin_counts,
    })
}

pub fn acp_dispatch_decision_json(
    session: &str,
    decision: &mvp::acp::AcpConversationDispatchDecision,
) -> Value {
    json!({
        "session": session,
        "decision": {
            "route_via_acp": decision.route_via_acp,
            "reason": decision.reason.as_str(),
            "automatic_routing_origin": decision
                .automatic_routing_origin
                .map(mvp::acp::AcpRoutingOrigin::as_str),
            "provenance": acp_dispatch_prediction_provenance_json(decision),
            "target": {
                "original_session_id": decision.target.original_session_id,
                "route_session_id": decision.target.route_session_id,
                "prefixed_agent_id": decision.target.prefixed_agent_id,
                "channel_id": decision.target.channel_id,
                "account_id": decision.target.account_id,
                "conversation_id": decision.target.conversation_id,
                "thread_id": decision.target.thread_id,
                "channel_path": decision.target.channel_path,
            }
        }
    })
}

pub fn acp_manager_observability_json(
    snapshot: &mvp::acp::AcpManagerObservabilitySnapshot,
) -> Value {
    json!({
        "runtime_cache": {
            "active_sessions": snapshot.runtime_cache.active_sessions,
            "idle_ttl_ms": snapshot.runtime_cache.idle_ttl_ms,
            "evicted_total": snapshot.runtime_cache.evicted_total,
            "last_evicted_at_ms": snapshot.runtime_cache.last_evicted_at_ms,
        },
        "sessions": {
            "bound": snapshot.sessions.bound,
            "unbound": snapshot.sessions.unbound,
            "activation_origin_counts": snapshot.sessions.activation_origin_counts,
            "provenance": {
                "surface": "session_activation_aggregate",
                "activation_origin_counts": snapshot.sessions.activation_origin_counts,
            },
            "backend_counts": snapshot.sessions.backend_counts,
        },
        "actors": {
            "active": snapshot.actors.active,
            "queue_depth": snapshot.actors.queue_depth,
            "waiting": snapshot.actors.waiting,
        },
        "turns": {
            "active": snapshot.turns.active,
            "queue_depth": snapshot.turns.queue_depth,
            "completed": snapshot.turns.completed,
            "failed": snapshot.turns.failed,
            "average_latency_ms": snapshot.turns.average_latency_ms,
            "max_latency_ms": snapshot.turns.max_latency_ms,
        },
        "errors_by_code": snapshot.errors_by_code,
    })
}

pub fn acp_event_summary_json(
    session: &str,
    limit: usize,
    summary: &mvp::acp::AcpTurnEventSummary,
) -> Value {
    json!({
        "session": session,
        "limit": limit,
        "provenance": acp_turn_provenance_json(summary),
        "summary": summary,
    })
}

pub fn format_acp_event_summary(
    session: &str,
    limit: usize,
    summary: &mvp::acp::AcpTurnEventSummary,
) -> String {
    format!(
        concat!(
            "acp_event_summary session={} limit={}\n",
            "records turn_event_records={} final_records={}\n",
            "events done={} error={} text={} usage_update={}\n",
            "turns succeeded={} cancelled={} failed={}\n",
            "latest backend_id={} agent_id={} routing_intent={} routing_origin={} session_key={} conversation_id={} binding_route_session_id={} channel_id={} account_id={} channel_conversation_id={} channel_thread_id={} trace_id={} source_message_id={} ack_cursor={} state={} stop_reason={} error={}\n",
            "rollup event_types={} stop_reasons={} routing_intents={} routing_origins={}\n"
        ),
        session,
        limit,
        summary.turn_event_records,
        summary.final_records,
        summary.done_events,
        summary.error_events,
        summary.text_events,
        summary.usage_update_events,
        summary.turns_succeeded,
        summary.turns_cancelled,
        summary.turns_failed,
        summary.last_backend_id.as_deref().unwrap_or("-"),
        summary.last_agent_id.as_deref().unwrap_or("-"),
        summary.last_routing_intent.as_deref().unwrap_or("-"),
        summary.last_routing_origin.as_deref().unwrap_or("-"),
        summary.last_session_key.as_deref().unwrap_or("-"),
        summary.last_conversation_id.as_deref().unwrap_or("-"),
        summary
            .last_binding_route_session_id
            .as_deref()
            .unwrap_or("-"),
        summary.last_channel_id.as_deref().unwrap_or("-"),
        summary.last_account_id.as_deref().unwrap_or("-"),
        summary
            .last_channel_conversation_id
            .as_deref()
            .unwrap_or("-"),
        summary.last_channel_thread_id.as_deref().unwrap_or("-"),
        summary.last_trace_id.as_deref().unwrap_or("-"),
        summary.last_source_message_id.as_deref().unwrap_or("-"),
        summary.last_ack_cursor.as_deref().unwrap_or("-"),
        summary.last_turn_state.as_deref().unwrap_or("-"),
        summary.last_stop_reason.as_deref().unwrap_or("-"),
        summary.last_error.as_deref().unwrap_or("-"),
        format_u32_rollup(&summary.event_type_counts),
        format_u32_rollup(&summary.stop_reason_counts),
        format_u32_rollup(&summary.routing_intent_counts),
        format_u32_rollup(&summary.routing_origin_counts)
    )
}

pub fn acp_session_mode_label(mode: mvp::acp::AcpSessionMode) -> &'static str {
    match mode {
        mvp::acp::AcpSessionMode::Interactive => "interactive",
        mvp::acp::AcpSessionMode::Background => "background",
        mvp::acp::AcpSessionMode::Review => "review",
    }
}

pub fn acp_session_state_label(state: mvp::acp::AcpSessionState) -> &'static str {
    match state {
        mvp::acp::AcpSessionState::Initializing => "initializing",
        mvp::acp::AcpSessionState::Ready => "ready",
        mvp::acp::AcpSessionState::Busy => "busy",
        mvp::acp::AcpSessionState::Cancelling => "cancelling",
        mvp::acp::AcpSessionState::Error => "error",
        mvp::acp::AcpSessionState::Closed => "closed",
    }
}
