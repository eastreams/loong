use std::collections::BTreeSet;
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use loong_app as mvp;
use loong_spec::CliResult;
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::acp_cli::acp_event_summary_json;
use crate::audit_cli::{self, AuditCommandOptions, AuditCommands};
use crate::sessions_cli::{self, SessionsCommandOptions, SessionsCommands};
use crate::status_cli;

const DEBUG_BUNDLE_SCHEMA_VERSION: u32 = 1;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum DebugCommands {
    /// Build one developer-facing debug bundle over runtime, provider, ACP, session, and audit signals
    Bundle {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = 12)]
        audit_limit: usize,
        #[arg(long, default_value_t = 20)]
        session_event_limit: usize,
        #[arg(long, default_value_t = 20)]
        history_limit: usize,
        #[arg(long, default_value_t = 200)]
        acp_event_limit: usize,
        #[arg(long, default_value_t = false)]
        include_history: bool,
    },
    /// Show one persisted debug bundle artifact in text or JSON form
    Show {
        #[arg(long)]
        artifact: String,
    },
}

#[derive(Debug, Clone)]
pub struct DebugCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub session: String,
    pub command: DebugCommands,
}

#[derive(Debug, Clone)]
pub struct DebugCommandExecution {
    pub resolved_config_path: String,
    pub current_session_id: String,
    pub payload: Value,
    pub artifact_path: Option<String>,
}

pub async fn run_debug_cli(options: DebugCommandOptions) -> CliResult<()> {
    let as_json = options.json;
    let execution = execute_debug_command(options).await?;
    if as_json {
        let pretty = serde_json::to_string_pretty(&execution.payload)
            .map_err(|error| format!("serialize debug CLI output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_debug_cli_text(&execution)?;
    println!("{rendered}");
    Ok(())
}

pub async fn execute_debug_command(
    options: DebugCommandOptions,
) -> CliResult<DebugCommandExecution> {
    let DebugCommandOptions {
        config,
        json: _,
        session,
        command,
    } = options;
    let (resolved_path, loaded_config) = mvp::config::load(config.as_deref())?;
    mvp::runtime_env::initialize_runtime_environment(&loaded_config, Some(&resolved_path));
    let resolved_config_path = resolved_path.display().to_string();
    let current_session_id = normalize_session_scope(session.as_str());

    let artifact_path = match &command {
        DebugCommands::Bundle { output, .. } => output
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        DebugCommands::Show { artifact } => Some(artifact.clone()),
    };

    let payload = match command {
        DebugCommands::Bundle {
            session_id,
            output,
            audit_limit,
            session_event_limit,
            history_limit,
            acp_event_limit,
            include_history,
        } => collect_debug_bundle(
            resolved_config_path.as_str(),
            &loaded_config,
            &current_session_id,
            session_id.as_deref(),
            audit_limit,
            session_event_limit,
            history_limit,
            acp_event_limit,
            include_history,
        )
        .await
        .and_then(|payload| {
            if let Some(output_path) = output
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                crate::persist_json_artifact(output_path, &payload, "debug bundle artifact")?;
            }
            Ok(payload)
        })?,
        DebugCommands::Show { artifact } => load_debug_bundle_artifact(Path::new(&artifact))?,
    };

    Ok(DebugCommandExecution {
        resolved_config_path,
        current_session_id,
        payload,
        artifact_path,
    })
}

async fn collect_debug_bundle(
    resolved_config_path: &str,
    config: &mvp::config::LoongConfig,
    current_session_id: &str,
    target_session_id: Option<&str>,
    audit_limit: usize,
    session_event_limit: usize,
    history_limit: usize,
    acp_event_limit: usize,
    include_history: bool,
) -> CliResult<Value> {
    let runtime_snapshot = crate::collect_runtime_snapshot_cli_state(Some(resolved_config_path))
        .map(|snapshot| crate::gateway::read_models::build_runtime_snapshot_read_model(&snapshot))
        .and_then(|snapshot| {
            serde_json::to_value(snapshot)
                .map_err(|error| format!("serialize runtime snapshot failed: {error}"))
        })?;
    let status = serde_json::to_value(
        status_cli::collect_status_cli_read_model(Some(resolved_config_path)).await?,
    )
    .map_err(|error| format!("serialize status debug section failed: {error}"))?;

    let mut errors = Vec::new();
    let session_status = collect_session_section(
        resolved_config_path,
        current_session_id,
        target_session_id,
        SessionsCommands::Status {
            session_id: target_session_id.unwrap_or_default().to_owned(),
        },
        "session_status",
        &mut errors,
    )
    .await;
    let session_events = collect_session_section(
        resolved_config_path,
        current_session_id,
        target_session_id,
        SessionsCommands::Events {
            session_id: target_session_id.unwrap_or_default().to_owned(),
            after_id: None,
            limit: session_event_limit.clamp(1, 200),
        },
        "session_events",
        &mut errors,
    )
    .await;
    let session_history = if include_history {
        collect_session_section(
            resolved_config_path,
            current_session_id,
            target_session_id,
            SessionsCommands::History {
                session_id: target_session_id.unwrap_or_default().to_owned(),
                limit: history_limit.clamp(1, 200),
            },
            "session_history",
            &mut errors,
        )
        .await
    } else {
        Value::Null
    };

    let acp_observability = match collect_acp_observability(config, resolved_config_path).await {
        Ok(value) => value,
        Err(error) => {
            errors.push(json!({
                "section": "acp_observability",
                "error": error,
            }));
            Value::Null
        }
    };

    let acp_event_summary =
        match collect_acp_event_summary(config, target_session_id, acp_event_limit) {
            Ok(value) => value,
            Err(error) => {
                errors.push(json!({
                    "section": "acp_event_summary",
                    "error": error,
                }));
                Value::Null
            }
        };

    let provider_failover_audit = collect_audit_section(
        resolved_config_path,
        AuditCommands::Recent {
            limit: audit_limit.clamp(1, 200),
            since_epoch_s: None,
            until_epoch_s: None,
            pack_id: None,
            agent_id: None,
            event_id: None,
            token_id: None,
            kind: Some("ProviderFailover".to_owned()),
            triage_label: None,
            query_contains: None,
            trust_tier: None,
        },
        "audit_provider_failover_recent",
        &mut errors,
    )?;
    let authorization_denied_audit = collect_audit_section(
        resolved_config_path,
        AuditCommands::Recent {
            limit: audit_limit.clamp(1, 200),
            since_epoch_s: None,
            until_epoch_s: None,
            pack_id: None,
            agent_id: None,
            event_id: None,
            token_id: None,
            kind: Some("AuthorizationDenied".to_owned()),
            triage_label: None,
            query_contains: None,
            trust_tier: None,
        },
        "audit_authorization_denied_recent",
        &mut errors,
    )?;
    let audit_summary = collect_audit_section(
        resolved_config_path,
        AuditCommands::Summary {
            limit: audit_limit.clamp(1, 200),
            since_epoch_s: None,
            until_epoch_s: None,
            pack_id: None,
            agent_id: None,
            event_id: None,
            token_id: None,
            kind: None,
            triage_label: None,
            group_by: None,
        },
        "audit_summary",
        &mut errors,
    )?;
    let audit_verify = collect_audit_section(
        resolved_config_path,
        AuditCommands::Verify,
        "audit_verify",
        &mut errors,
    )?;

    let recipes = json!([
        {
            "label": "recent provider failovers",
            "command": format!("loong audit recent --config '{}' --kind ProviderFailover --limit {}", resolved_config_path, audit_limit.clamp(1, 200)),
        },
        {
            "label": "ACP observability snapshot",
            "command": format!("loong acp-observability --config '{}'", resolved_config_path),
        },
        {
            "label": "runtime snapshot artifact",
            "command": format!("loong runtime-snapshot --config '{}'", resolved_config_path),
        },
        {
            "label": "session inspection",
            "command": target_session_id
                .map(|session_id| {
                    format!(
                        "loong sessions --config '{}' --session '{}' status '{}'",
                        resolved_config_path, current_session_id, session_id
                    )
                })
                .unwrap_or_else(|| {
                    format!(
                        "loong sessions --config '{}' --session '{}' list --limit 20",
                        resolved_config_path, current_session_id
                    )
                }),
        }
    ]);
    let lineage = build_debug_bundle_lineage(current_session_id, target_session_id)?;
    let correlation_index = build_debug_correlation_index(
        current_session_id,
        target_session_id,
        &runtime_snapshot,
        &acp_event_summary,
        &session_status,
        &provider_failover_audit,
    );
    let attention_hints = build_debug_attention_hints(
        &session_status,
        &acp_observability,
        &provider_failover_audit,
        &audit_verify,
        errors.as_slice(),
    );

    Ok(json!({
        "schema": {
            "version": DEBUG_BUNDLE_SCHEMA_VERSION,
            "surface": "debug_bundle",
            "purpose": "developer_debug_capture",
        },
        "lineage": lineage,
        "config": resolved_config_path,
        "scope_session_id": current_session_id,
        "target_session_id": target_session_id,
        "runtime_snapshot": runtime_snapshot,
        "status": status,
        "acp_observability": acp_observability,
        "acp_event_summary": acp_event_summary,
        "session_status": session_status,
        "session_events": session_events,
        "session_history": session_history,
        "audit_summary": audit_summary,
        "audit_verify": audit_verify,
        "audit_provider_failover_recent": provider_failover_audit,
        "audit_authorization_denied_recent": authorization_denied_audit,
        "debug": {
            "correlation_index": correlation_index,
            "attention_hints": attention_hints,
        },
        "recipes": recipes,
        "errors": errors,
    }))
}

async fn collect_session_section(
    resolved_config_path: &str,
    current_session_id: &str,
    target_session_id: Option<&str>,
    command: SessionsCommands,
    section_name: &str,
    errors: &mut Vec<Value>,
) -> Value {
    let Some(_target_session_id) = target_session_id else {
        return Value::Null;
    };

    match sessions_cli::execute_sessions_command(SessionsCommandOptions {
        config: Some(resolved_config_path.to_owned()),
        json: false,
        session: current_session_id.to_owned(),
        command,
    })
    .await
    {
        Ok(execution) => execution.payload,
        Err(error) => {
            errors.push(json!({
                "section": section_name,
                "error": error,
            }));
            Value::Null
        }
    }
}

fn collect_audit_section(
    resolved_config_path: &str,
    command: AuditCommands,
    section_name: &str,
    errors: &mut Vec<Value>,
) -> CliResult<Value> {
    match audit_cli::execute_audit_command(AuditCommandOptions {
        config: Some(resolved_config_path.to_owned()),
        json: false,
        command,
    }) {
        Ok(execution) => Ok(audit_cli::audit_cli_json(&execution)),
        Err(error) => {
            errors.push(json!({
                "section": section_name,
                "error": error,
            }));
            Ok(Value::Null)
        }
    }
}

async fn collect_acp_observability(
    config: &mvp::config::LoongConfig,
    resolved_config_path: &str,
) -> CliResult<Value> {
    let manager = mvp::acp::shared_acp_session_manager(config)?;
    let snapshot = manager.observability_snapshot(config).await?;
    let read_model = crate::gateway::read_models::build_acp_observability_read_model(
        resolved_config_path,
        &snapshot,
    );
    serde_json::to_value(read_model)
        .map_err(|error| format!("serialize ACP observability debug section failed: {error}"))
}

fn collect_acp_event_summary(
    config: &mvp::config::LoongConfig,
    session_id: Option<&str>,
    limit: usize,
) -> CliResult<Value> {
    let Some(session_id) = session_id else {
        return Ok(Value::Null);
    };
    if limit == 0 {
        return Err("debug bundle acp_event_limit must be >= 1".to_owned());
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        let turns = mvp::memory::window_direct(session_id, limit, &mem_config)
            .map_err(|error| format!("load ACP event summary failed: {error}"))?;
        let summary = mvp::acp::summarize_turn_events(
            turns
                .iter()
                .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str())),
        );
        Ok(acp_event_summary_json(session_id, limit, &summary))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (config, limit, session_id);
        Err("debug bundle ACP event summary requires memory-sqlite feature".to_owned())
    }
}

fn normalize_session_scope(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn build_debug_bundle_lineage(
    current_session_id: &str,
    target_session_id: Option<&str>,
) -> CliResult<Value> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("build debug bundle lineage failed: {error}"))?;
    let created_at = OffsetDateTime::from_unix_timestamp_nanos(now.as_nanos() as i128)
        .map_err(|error| format!("format debug bundle timestamp failed: {error}"))?;
    let created_at = created_at
        .format(&Rfc3339)
        .map_err(|error| format!("encode debug bundle timestamp failed: {error}"))?;
    let bundle_id = format!("debug-bundle-{}-{}", now.as_millis(), process::id());

    Ok(json!({
        "bundle_id": bundle_id,
        "created_at": created_at,
        "command_kind": "debug_bundle",
        "entrypoint": "cli",
        "scope_session_id": current_session_id,
        "target_session_id": target_session_id,
    }))
}

fn build_debug_correlation_index(
    current_session_id: &str,
    target_session_id: Option<&str>,
    runtime_snapshot: &Value,
    acp_event_summary: &Value,
    session_status: &Value,
    provider_failover_audit: &Value,
) -> Value {
    let mut session_ids = BTreeSet::new();
    let mut trace_ids = BTreeSet::new();
    let mut route_session_ids = BTreeSet::new();
    let mut provider_ids = BTreeSet::new();
    let mut provider_request_ids = BTreeSet::new();
    let mut auth_error_codes = BTreeSet::new();
    let mut conversation_ids = BTreeSet::new();
    let mut channel_ids = BTreeSet::new();
    let mut account_ids = BTreeSet::new();
    let mut thread_ids = BTreeSet::new();
    let mut backend_ids = BTreeSet::new();
    let mut audit_event_ids = BTreeSet::new();

    session_ids.insert(current_session_id.to_owned());
    if let Some(target_session_id) = target_session_id {
        session_ids.insert(target_session_id.to_owned());
    }
    collect_json_path_str(session_status, &["session", "session_id"], &mut session_ids);
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_trace_id"],
        &mut trace_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_binding_route_session_id"],
        &mut route_session_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_conversation_id"],
        &mut conversation_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_channel_id"],
        &mut channel_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_account_id"],
        &mut account_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_channel_thread_id"],
        &mut thread_ids,
    );
    collect_json_path_str(
        acp_event_summary,
        &["summary", "last_backend_id"],
        &mut backend_ids,
    );
    collect_json_path_str(
        runtime_snapshot,
        &["provider", "active_profile_id"],
        &mut provider_ids,
    );

    if let Some(events) = provider_failover_audit
        .get("events")
        .and_then(Value::as_array)
    {
        for event in events {
            collect_json_path_str(event, &["event_id"], &mut audit_event_ids);
            collect_json_path_str(
                event,
                &["kind", "ProviderFailover", "provider_id"],
                &mut provider_ids,
            );
            collect_json_path_str(
                event,
                &["kind", "ProviderFailover", "request_id"],
                &mut provider_request_ids,
            );
            collect_json_path_str(
                event,
                &["kind", "ProviderFailover", "auth_error_code"],
                &mut auth_error_codes,
            );
        }
    }

    json!({
        "session_ids": set_to_json_array(session_ids),
        "trace_ids": set_to_json_array(trace_ids),
        "route_session_ids": set_to_json_array(route_session_ids),
        "provider_ids": set_to_json_array(provider_ids),
        "provider_request_ids": set_to_json_array(provider_request_ids),
        "auth_error_codes": set_to_json_array(auth_error_codes),
        "conversation_ids": set_to_json_array(conversation_ids),
        "channel_ids": set_to_json_array(channel_ids),
        "account_ids": set_to_json_array(account_ids),
        "thread_ids": set_to_json_array(thread_ids),
        "backend_ids": set_to_json_array(backend_ids),
        "audit_event_ids": set_to_json_array(audit_event_ids),
    })
}

fn build_debug_attention_hints(
    session_status: &Value,
    acp_observability: &Value,
    provider_failover_audit: &Value,
    audit_verify: &Value,
    errors: &[Value],
) -> Value {
    let mut hints = Vec::new();

    let provider_failover_count = count_json_array(provider_failover_audit.pointer("/events"));
    if provider_failover_count > 0 {
        hints.push(format!(
            "provider_failover_present count={} inspect request_id/auth_error_code first",
            provider_failover_count
        ));
    }

    let turn_failures =
        json_path_u64(acp_observability, &["snapshot", "turns", "failed"]).unwrap_or(0);
    if turn_failures > 0 {
        hints.push(format!(
            "acp_turn_failures count={} inspect ACP event summary and route session correlation",
            turn_failures
        ));
    }

    let last_error = json_path_str(session_status, &["session", "last_error"]);
    let last_error = last_error.filter(|value| !value.is_empty());
    if let Some(last_error) = last_error {
        hints.push(format!(
            "session_last_error present={} inspect session events and terminal outcome",
            last_error
        ));
    }

    let terminal_outcome_state = json_path_str(session_status, &["terminal_outcome_state"]);
    let terminal_outcome_state = terminal_outcome_state.filter(|value| *value == "missing");
    if terminal_outcome_state.is_some() {
        hints.push(
            "session_terminal_outcome missing inspect recovery source and recent events".to_owned(),
        );
    }

    let audit_verify_outcome = json_path_str(audit_verify, &["outcome"]);
    let audit_verify_outcome = audit_verify_outcome.filter(|value| *value != "healthy");
    if let Some(audit_verify_outcome) = audit_verify_outcome {
        hints.push(format!(
            "audit_verify outcome={} inspect audit journal integrity before trusting incident evidence",
            audit_verify_outcome
        ));
    }

    if !errors.is_empty() {
        hints.push(format!(
            "bundle_collection_errors count={} inspect errors[] before assuming the bundle is complete",
            errors.len()
        ));
    }

    json!(hints)
}

pub fn render_debug_cli_text(execution: &DebugCommandExecution) -> CliResult<String> {
    render_debug_payload_text(
        &execution.payload,
        execution.resolved_config_path.as_str(),
        execution.current_session_id.as_str(),
        execution.artifact_path.as_deref(),
    )
}

fn render_debug_payload_text(
    payload: &Value,
    resolved_config_path: &str,
    current_session_id: &str,
    artifact_path: Option<&str>,
) -> CliResult<String> {
    let mut lines = vec![format!(
        "🔎 debug bundle config={} scope_session={} target_session={}",
        resolved_config_path,
        current_session_id,
        payload
            .get("target_session_id")
            .and_then(Value::as_str)
            .unwrap_or("-")
    )];
    if let Some(artifact_path) = artifact_path {
        lines.push(format!("📦 artifact {}", artifact_path));
    }

    let status = payload.get("status").unwrap_or(&Value::Null);
    lines.push(format!(
        "🧭 runtime active_provider={} active_model={} memory_profile={}",
        json_path_str(status, &["active_provider"]).unwrap_or("-"),
        json_path_str(status, &["active_model"]).unwrap_or("-"),
        json_path_str(status, &["memory_profile"]).unwrap_or("-"),
    ));
    lines.push(format!(
        "🧩 correlation traces={} route_sessions={} provider_requests={}",
        count_json_array(payload.pointer("/debug/correlation_index/trace_ids")),
        count_json_array(payload.pointer("/debug/correlation_index/route_session_ids")),
        count_json_array(payload.pointer("/debug/correlation_index/provider_request_ids")),
    ));

    let provider_transport = payload
        .get("runtime_snapshot")
        .and_then(|value| value.get("provider"))
        .and_then(|value| value.get("transport_runtime"))
        .unwrap_or(&Value::Null);
    lines.push(format!(
        "🔌 provider cache entries={} hits={} misses={} built={} failover_total={} continued={} exhausted={}",
        json_path_u64(provider_transport, &["http_client_cache_entries"]).unwrap_or(0),
        json_path_u64(provider_transport, &["http_client_cache_hits"]).unwrap_or(0),
        json_path_u64(provider_transport, &["http_client_cache_misses"]).unwrap_or(0),
        json_path_u64(provider_transport, &["built_http_clients"]).unwrap_or(0),
        json_path_u64(provider_transport, &["failover_total_events"]).unwrap_or(0),
        json_path_u64(provider_transport, &["failover_continued_events"]).unwrap_or(0),
        json_path_u64(provider_transport, &["failover_exhausted_events"]).unwrap_or(0),
    ));
    lines.push(format!(
        "  reasons={} stages={} providers={}",
        format_value_rollup(provider_transport.get("failover_by_reason")),
        format_value_rollup(provider_transport.get("failover_by_stage")),
        format_value_rollup(provider_transport.get("failover_by_provider")),
    ));

    let acp = payload.get("acp_observability").unwrap_or(&Value::Null);
    lines.push(format!(
        "🔁 acp active_sessions={} bound={} unbound={} actor_queue={} turn_queue={} turn_failures={} error_total={}",
        json_path_u64(acp, &["snapshot", "runtime_cache", "active_sessions"]).unwrap_or(0),
        json_path_u64(acp, &["snapshot", "sessions", "bound"]).unwrap_or(0),
        json_path_u64(acp, &["snapshot", "sessions", "unbound"]).unwrap_or(0),
        json_path_u64(acp, &["snapshot", "actors", "queue_depth"]).unwrap_or(0),
        json_path_u64(acp, &["snapshot", "turns", "queue_depth"]).unwrap_or(0),
        json_path_u64(acp, &["snapshot", "turns", "failed"]).unwrap_or(0),
        count_json_object_keys(acp.pointer("/snapshot/errors_by_code")),
    ));

    let session_status = payload.get("session_status").unwrap_or(&Value::Null);
    if !session_status.is_null() {
        lines.push(format!(
            "🧠 session state={} turns={} last_error={}",
            json_path_str(session_status, &["session", "state"]).unwrap_or("-"),
            json_path_u64(session_status, &["session", "turn_count"]).unwrap_or(0),
            json_path_str(session_status, &["session", "last_error"]).unwrap_or("-"),
        ));
        lines.push(format!(
            "  recent_events={} terminal_outcome_state={} recovery_source={}",
            count_json_array(session_status.pointer("/recent_events")),
            json_path_str(session_status, &["terminal_outcome_state"]).unwrap_or("-"),
            json_path_str(session_status, &["recovery", "source"]).unwrap_or("-"),
        ));
    }

    lines.push(format!(
        "🪵 audit provider_failover_events={} authorization_denied_events={} summary_events={} verify_status={}",
        count_json_array(payload.pointer("/audit_provider_failover_recent/events")),
        count_json_array(payload.pointer("/audit_authorization_denied_recent/events")),
        json_path_u64(payload, &["audit_summary", "loaded_events"]).unwrap_or(0),
        json_path_str(payload, &["audit_verify", "outcome"]).unwrap_or("-"),
    ));
    if let Some(line) =
        latest_provider_failover_line(payload.pointer("/audit_provider_failover_recent/events"))
    {
        lines.push(format!("  latest_provider_failover={line}"));
    }

    let errors = payload
        .get("errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let attention_hints = payload
        .get("debug")
        .and_then(|debug| debug.get("attention_hints"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if errors.is_empty() {
        lines.push(format!("⚠️ attention {}", attention_hints.len()));
    } else {
        lines.push(format!(
            "⚠️ attention {}",
            errors.len() + attention_hints.len()
        ));
        for error in errors.iter().take(5) {
            lines.push(format!(
                "  - section={} error={}",
                error.get("section").and_then(Value::as_str).unwrap_or("-"),
                error.get("error").and_then(Value::as_str).unwrap_or("-"),
            ));
        }
    }
    for hint in attention_hints.iter().take(5) {
        lines.push(format!("  - hint={}", hint.as_str().unwrap_or("-"),));
    }

    lines.push("💡 next steps".to_owned());
    if let Some(recipes) = payload.get("recipes").and_then(Value::as_array) {
        for recipe in recipes {
            lines.push(format!(
                "  - {}: {}",
                recipe
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("recipe"),
                recipe.get("command").and_then(Value::as_str).unwrap_or("-"),
            ));
        }
    }

    Ok(lines.join("\n"))
}

fn load_debug_bundle_artifact(artifact_path: &Path) -> CliResult<Value> {
    let encoded = std::fs::read_to_string(artifact_path).map_err(|error| {
        format!(
            "read debug bundle artifact {} failed: {error}",
            artifact_path.display()
        )
    })?;
    let payload = serde_json::from_str::<Value>(&encoded).map_err(|error| {
        format!(
            "parse debug bundle artifact {} failed: {error}",
            artifact_path.display()
        )
    })?;

    let surface = payload
        .get("schema")
        .and_then(|schema| schema.get("surface"))
        .and_then(Value::as_str);
    if surface != Some("debug_bundle") {
        return Err(format!(
            "debug bundle artifact {} uses unsupported surface {:?}",
            artifact_path.display(),
            surface
        ));
    }

    Ok(payload)
}

fn json_path_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str()
}

fn json_path_u64(value: &Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_u64()
}

fn collect_json_path_str(value: &Value, path: &[&str], target: &mut BTreeSet<String>) {
    let Some(found) = json_path_str(value, path) else {
        return;
    };
    if found.is_empty() {
        return;
    }
    target.insert(found.to_owned());
}

fn set_to_json_array(values: BTreeSet<String>) -> Value {
    let ordered = values.into_iter().collect::<Vec<_>>();
    json!(ordered)
}

fn count_json_object_keys(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_object)
        .map(|object| object.len())
        .unwrap_or(0)
}

fn count_json_array(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0)
}

fn format_value_rollup(value: Option<&Value>) -> String {
    let Some(object) = value.and_then(Value::as_object) else {
        return "-".to_owned();
    };
    if object.is_empty() {
        return "-".to_owned();
    }

    object
        .iter()
        .map(|(key, value)| format!("{key}={}", value.as_u64().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(",")
}

fn latest_provider_failover_line(value: Option<&Value>) -> Option<String> {
    let event = value.and_then(Value::as_array)?.last()?;
    Some(format!(
        "provider_id={} reason={} model={} request_id={} auth_error_code={}",
        event
            .pointer("/kind/ProviderFailover/provider_id")
            .or_else(|| event.get("provider_id"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        event
            .pointer("/kind/ProviderFailover/reason")
            .or_else(|| event.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        event
            .pointer("/kind/ProviderFailover/model")
            .or_else(|| event.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        event
            .pointer("/kind/ProviderFailover/request_id")
            .or_else(|| event.get("request_id"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
        event
            .pointer("/kind/ProviderFailover/auth_error_code")
            .or_else(|| event.get("auth_error_code"))
            .and_then(Value::as_str)
            .unwrap_or("-"),
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DebugCommandExecution, build_debug_correlation_index, load_debug_bundle_artifact,
        normalize_session_scope, render_debug_cli_text,
    };
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn normalize_session_scope_defaults_when_empty() {
        assert_eq!(normalize_session_scope(""), "default");
        assert_eq!(normalize_session_scope("  "), "default");
        assert_eq!(normalize_session_scope("debug-session"), "debug-session");
    }

    #[test]
    fn render_debug_cli_text_uses_bundle_sections_and_emoji_headings() {
        let execution = DebugCommandExecution {
            resolved_config_path: "/tmp/loong.toml".to_owned(),
            current_session_id: "default".to_owned(),
            artifact_path: Some("/tmp/debug-bundle.json".to_owned()),
            payload: json!({
                "target_session_id": "session-1",
                "status": {
                    "active_provider": "openai",
                    "active_model": "gpt-5",
                    "memory_profile": "window_plus_summary",
                },
                "runtime_snapshot": {
                    "provider": {
                        "transport_runtime": {
                            "http_client_cache_entries": 1,
                            "http_client_cache_hits": 2,
                            "http_client_cache_misses": 3,
                            "built_http_clients": 1,
                            "failover_total_events": 4,
                            "failover_continued_events": 3,
                            "failover_exhausted_events": 1,
                            "failover_by_reason": { "rate_limited": 2 },
                            "failover_by_stage": { "status_failure": 2 },
                            "failover_by_provider": { "openai": 4 }
                        }
                    }
                },
                "acp_observability": {
                    "snapshot": {
                        "runtime_cache": { "active_sessions": 2 },
                        "sessions": { "bound": 1, "unbound": 1 },
                        "actors": { "queue_depth": 3 },
                        "turns": { "queue_depth": 4, "failed": 1 },
                        "errors_by_code": { "timeout": 1 }
                    }
                },
                "session_status": {
                    "session": {
                        "state": "failed",
                        "turn_count": 7,
                        "last_error": "request timeout"
                    },
                    "terminal_outcome_state": "missing",
                    "recovery": { "source": "last_error" },
                    "recent_events": [{ "id": 1 }]
                },
                "audit_provider_failover_recent": {
                    "events": [
                        {
                            "event_id": "evt-1",
                            "kind": {
                                "ProviderFailover": {
                                    "provider_id": "openai",
                                    "reason": "rate_limited",
                                    "model": "gpt-5",
                                    "request_id": "req-123",
                                    "auth_error_code": "token_expired"
                                }
                            }
                        }
                    ]
                },
                "audit_authorization_denied_recent": {
                    "events": []
                },
                "audit_summary": {
                    "loaded_events": 3
                },
                "audit_verify": {
                    "outcome": "healthy"
                },
                "debug": {
                    "correlation_index": {
                        "trace_ids": ["trace-123"]
                    },
                    "attention_hints": [
                        "provider_failover_present count=1 inspect request_id/auth_error_code first"
                    ]
                },
                "recipes": [
                    { "label": "provider failovers", "command": "loong audit recent --kind ProviderFailover" }
                ],
                "errors": []
            }),
        };

        let rendered = render_debug_cli_text(&execution).expect("render debug bundle");
        assert!(rendered.contains("🔎 debug bundle"));
        assert!(rendered.contains("🔌 provider"));
        assert!(rendered.contains("🔁 acp"));
        assert!(rendered.contains("🧠 session"));
        assert!(rendered.contains("🧩 correlation"));
        assert!(rendered.contains("🪵 audit"));
        assert!(rendered.contains("📦 artifact /tmp/debug-bundle.json"));
        assert!(rendered.contains("💡 next steps"));
        assert!(rendered.contains("hint=provider_failover_present"));
        assert!(rendered.contains("request_id=req-123"));
    }

    #[test]
    fn build_debug_correlation_index_collects_deduplicated_identifiers() {
        let correlation = build_debug_correlation_index(
            "root-session",
            Some("target-session"),
            &json!({
                "provider": {
                    "active_profile_id": "openai"
                }
            }),
            &json!({
                "summary": {
                    "last_trace_id": "trace-123",
                    "last_binding_route_session_id": "route-123",
                    "last_conversation_id": "conv-123",
                    "last_channel_id": "channel-123",
                    "last_account_id": "acct-123",
                    "last_channel_thread_id": "thread-123",
                    "last_backend_id": "backend-123"
                }
            }),
            &json!({
                "session": {
                    "session_id": "target-session"
                }
            }),
            &json!({
                "events": [
                    {
                        "event_id": "evt-1",
                        "kind": {
                            "ProviderFailover": {
                                "provider_id": "openai",
                                "request_id": "req-123",
                                "auth_error_code": "token_expired"
                            }
                        }
                    }
                ]
            }),
        );

        assert_eq!(
            correlation["session_ids"],
            json!(["root-session", "target-session"])
        );
        assert_eq!(correlation["trace_ids"], json!(["trace-123"]));
        assert_eq!(correlation["route_session_ids"], json!(["route-123"]));
        assert_eq!(correlation["provider_ids"], json!(["openai"]));
        assert_eq!(correlation["provider_request_ids"], json!(["req-123"]));
        assert_eq!(correlation["auth_error_codes"], json!(["token_expired"]));
        assert_eq!(correlation["audit_event_ids"], json!(["evt-1"]));
    }

    #[test]
    fn load_debug_bundle_artifact_reads_valid_payload() {
        let temp = tempdir().expect("create temp dir");
        let artifact_path = temp.path().join("debug-bundle.json");
        fs::write(
            &artifact_path,
            serde_json::to_string_pretty(&json!({
                "schema": {
                    "version": 1,
                    "surface": "debug_bundle",
                    "purpose": "developer_debug_capture",
                },
                "config": "/tmp/loong.toml"
            }))
            .expect("serialize artifact"),
        )
        .expect("write artifact");

        let payload = load_debug_bundle_artifact(&artifact_path).expect("load artifact");
        assert_eq!(payload["schema"]["surface"], "debug_bundle");
    }

    #[test]
    fn load_debug_bundle_artifact_rejects_wrong_surface() {
        let temp = tempdir().expect("create temp dir");
        let artifact_path = temp.path().join("not-debug.json");
        fs::write(
            &artifact_path,
            serde_json::to_string_pretty(&json!({
                "schema": {
                    "version": 1,
                    "surface": "runtime_snapshot",
                    "purpose": "experiment_reproducibility",
                }
            }))
            .expect("serialize artifact"),
        )
        .expect("write artifact");

        let error = load_debug_bundle_artifact(&artifact_path).expect_err("reject wrong surface");
        assert!(error.contains("unsupported surface"));
    }
}
