#[cfg(feature = "memory-sqlite")]
use std::time::{SystemTime, UNIX_EPOCH};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{json, Value};

use crate::config::{SessionVisibility, ToolConfig};
use crate::memory;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{
    build_queued_async_overdue_recovery_payload, observe_missing_recovery, recovery_json,
    SessionRecoveryRecord, RECOVERY_EVENT_KIND, RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
};

#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    SessionEventRecord, SessionKind, SessionObservationRecord, SessionRepository, SessionState,
    SessionSummaryRecord, SessionTerminalOutcomeRecord,
};

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionInspectionSnapshot {
    pub session: SessionSummaryRecord,
    pub terminal_outcome: Option<SessionTerminalOutcomeRecord>,
    pub recent_events: Vec<SessionEventRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionObservationSnapshot {
    pub inspection: SessionInspectionSnapshot,
    pub tail_events: Vec<SessionEventRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionDelegateLifecycleRecord {
    mode: &'static str,
    phase: &'static str,
    queued_at: Option<i64>,
    started_at: Option<i64>,
    timeout_seconds: Option<u64>,
    staleness: Option<SessionDelegateStalenessRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionDelegateStalenessRecord {
    state: &'static str,
    reference: &'static str,
    elapsed_seconds: u64,
    threshold_seconds: u64,
    deadline_at: i64,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionRecoverPlan {
    queued_at: i64,
    elapsed_seconds: u64,
    timeout_seconds: u64,
    deadline_at: i64,
}

#[cfg(test)]
pub fn execute_session_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_session_tool_with_policies(request, current_session_id, config, &ToolConfig::default())
}

pub fn execute_session_tool_with_policies(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, current_session_id, config, tool_config);
        return Err(
            "session tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: session tools are disabled by config".to_owned());
        }
        match request.tool_name.as_str() {
            "sessions_list" => execute_sessions_list(current_session_id, config, tool_config),
            "session_events" => {
                execute_session_events(request.payload, current_session_id, config, tool_config)
            }
            "sessions_history" => {
                execute_sessions_history(request.payload, current_session_id, config, tool_config)
            }
            "session_status" => {
                execute_session_status(request.payload, current_session_id, config, tool_config)
            }
            "session_recover" => {
                execute_session_recover(request.payload, current_session_id, config, tool_config)
            }
            other => Err(format!(
                "app_tool_not_found: unknown session tool `{other}`"
            )),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn execute_sessions_list(
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let mut sessions = repo.list_visible_sessions(current_session_id)?;
    if tool_config.sessions.visibility == SessionVisibility::SelfOnly {
        sessions.retain(|session| session.session_id == current_session_id);
    }
    sessions.truncate(tool_config.sessions.list_limit);
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "sessions": sessions.into_iter().map(session_summary_json).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_events(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id")?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let events = match after_id {
        Some(after_id) => repo.list_events_after(&target_session_id, after_id.max(0), limit)?,
        None => repo.list_recent_events(&target_session_id, limit)?,
    };
    let next_after_id = events
        .last()
        .map(|event| event.id)
        .unwrap_or(after_id.unwrap_or(0));

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "session_id": target_session_id,
            "after_id": after_id,
            "limit": limit,
            "next_after_id": next_after_id,
            "events": events.into_iter().map(session_event_json).collect::<Vec<_>>(),
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_sessions_history(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id")?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let turns = memory::window_direct(&target_session_id, limit, config)
        .map_err(|error| format!("load session transcript failed: {error}"))?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "session_id": target_session_id,
            "limit": limit,
            "turns": turns,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_status(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id")?;
    let snapshot = inspect_visible_session_with_policies(
        &target_session_id,
        current_session_id,
        config,
        tool_config,
        5,
    )?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_inspection_payload(snapshot),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_recover(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let target_session_id = required_payload_string(&payload, "session_id")?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let snapshot = inspect_visible_session_with_policies(
        &target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    let recover_plan = build_session_recover_plan(&snapshot, current_unix_ts())?;
    let recovery_error = format!(
        "delegate_async_queued_overdue_marked_failed: queued delegate child exceeded timeout after {}s (threshold {}s)",
        recover_plan.elapsed_seconds, recover_plan.timeout_seconds
    );
    let outcome = crate::tools::delegate::delegate_error_outcome(
        snapshot.session.session_id.clone(),
        snapshot.session.label.clone(),
        recovery_error.clone(),
        recover_plan.elapsed_seconds.saturating_mul(1_000),
    );
    let finalized = repo.finalize_session_terminal_if_current(
        &target_session_id,
        SessionState::Ready,
        crate::session::repository::FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some(recovery_error.clone()),
            event_kind: RECOVERY_EVENT_KIND.to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            event_payload_json: build_queued_async_overdue_recovery_payload(
                snapshot.session.label.as_deref(),
                recover_plan.queued_at,
                recover_plan.elapsed_seconds,
                recover_plan.timeout_seconds,
                recover_plan.deadline_at,
                &recovery_error,
            ),
            outcome_status: outcome.status.clone(),
            outcome_payload_json: outcome.payload.clone(),
        },
    )?;
    if finalized.is_none() {
        let latest = repo
            .load_session_summary_with_legacy_fallback(&target_session_id)?
            .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        return Err(format!(
            "session_recover_state_changed: session `{target_session_id}` is no longer recoverable from state `{}`",
            latest.state.as_str()
        ));
    }
    let recovered_snapshot = inspect_visible_session_with_policies(
        &target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    let mut payload = session_inspection_payload(recovered_snapshot);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "recovery_action".to_owned(),
            json!({
                "kind": RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
                "previous_state": "ready",
                "next_state": "failed",
                "reference": "queued",
                "elapsed_seconds": recover_plan.elapsed_seconds,
                "timeout_seconds": recover_plan.timeout_seconds,
                "deadline_at": recover_plan.deadline_at,
            }),
        );
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn inspect_visible_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    recent_event_limit: usize,
) -> Result<SessionInspectionSnapshot, String> {
    Ok(observe_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        recent_event_limit,
        None,
        0,
    )?
    .inspection)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn observe_visible_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    recent_event_limit: usize,
    tail_after_id: Option<i64>,
    tail_page_limit: usize,
) -> Result<SessionObservationSnapshot, String> {
    let target_session_id = normalize_required_session_id(target_session_id)?;
    let repo = SessionRepository::new(config)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let SessionObservationRecord {
        session,
        terminal_outcome,
        recent_events,
        tail_events,
    } = repo
        .load_session_observation(
            &target_session_id,
            recent_event_limit,
            tail_after_id,
            tail_page_limit,
        )?
        .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;

    Ok(SessionObservationSnapshot {
        inspection: SessionInspectionSnapshot {
            session,
            terminal_outcome,
            recent_events,
        },
        tail_events,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_state_is_terminal(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Completed | SessionState::Failed | SessionState::TimedOut
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_inspection_payload(snapshot: SessionInspectionSnapshot) -> Value {
    let terminal_outcome_state =
        session_terminal_outcome_state(snapshot.session.state, snapshot.terminal_outcome.is_some());
    let delegate_lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.recent_events.as_slice(),
        current_unix_ts(),
    );
    let recovery = match terminal_outcome_state {
        "missing" => Some(observe_missing_recovery(
            snapshot.recent_events.as_slice(),
            snapshot.session.last_error.as_deref(),
        )),
        _ => None,
    };
    let terminal_outcome_missing_reason = match terminal_outcome_state {
        "missing" => session_terminal_outcome_missing_reason(recovery.as_ref()),
        _ => None,
    };
    json!({
        "session": {
            "session_id": snapshot.session.session_id,
            "kind": snapshot.session.kind.as_str(),
            "parent_session_id": snapshot.session.parent_session_id,
            "label": snapshot.session.label,
            "state": snapshot.session.state.as_str(),
            "created_at": snapshot.session.created_at,
            "updated_at": snapshot.session.updated_at,
            "last_error": snapshot.session.last_error,
        },
        "terminal_outcome_state": terminal_outcome_state,
        "terminal_outcome_missing_reason": terminal_outcome_missing_reason,
        "delegate_lifecycle": delegate_lifecycle.map(session_delegate_lifecycle_json),
        "recovery": recovery.map(recovery_json),
        "terminal_outcome": snapshot.terminal_outcome.map(session_terminal_outcome_json),
        "recent_events": snapshot
            .recent_events
            .into_iter()
            .map(session_event_json)
            .collect::<Vec<_>>(),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_state(state: SessionState, has_terminal_outcome: bool) -> &'static str {
    if has_terminal_outcome {
        "present"
    } else if session_state_is_terminal(state) {
        "missing"
    } else {
        "not_terminal"
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_missing_reason(
    recovery: Option<&SessionRecoveryRecord>,
) -> Option<String> {
    recovery.map(|recovery| recovery.kind.clone())
}

#[cfg(feature = "memory-sqlite")]
fn build_session_recover_plan(
    snapshot: &SessionInspectionSnapshot,
    now_ts: i64,
) -> Result<SessionRecoverPlan, String> {
    if snapshot.session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session_recover_not_supported: session `{}` is not a delegate child",
            snapshot.session.session_id
        ));
    }
    if snapshot.terminal_outcome.is_some() || session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is already terminal",
            snapshot.session.session_id
        ));
    }
    let lifecycle =
        session_delegate_lifecycle_at(&snapshot.session, snapshot.recent_events.as_slice(), now_ts)
            .ok_or_else(|| {
                format!(
            "session_recover_not_recoverable: session `{}` is missing delegate lifecycle metadata",
            snapshot.session.session_id
        )
            })?;
    if lifecycle.mode != "async"
        || lifecycle.phase != "queued"
        || snapshot.session.state != SessionState::Ready
    {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is not an overdue queued async child",
            snapshot.session.session_id
        ));
    }
    let queued_at = lifecycle.queued_at.ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing queued timestamp",
            snapshot.session.session_id
        )
    })?;
    let staleness = lifecycle.staleness.ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing staleness metadata",
            snapshot.session.session_id
        )
    })?;
    if staleness.state != "overdue" {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is not overdue",
            snapshot.session.session_id
        ));
    }
    let timeout_seconds = lifecycle.timeout_seconds.ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing timeout metadata",
            snapshot.session.session_id
        )
    })?;

    Ok(SessionRecoverPlan {
        queued_at,
        elapsed_seconds: staleness.elapsed_seconds,
        timeout_seconds,
        deadline_at: staleness.deadline_at,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_delegate_lifecycle_at(
    session: &SessionSummaryRecord,
    recent_events: &[SessionEventRecord],
    now_ts: i64,
) -> Option<SessionDelegateLifecycleRecord> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let mut queued_at = None;
    let mut started_at = None;
    let mut queued_timeout_seconds = None;
    let mut started_timeout_seconds = None;
    for event in recent_events {
        match event.event_kind.as_str() {
            "delegate_queued" => {
                queued_at = Some(event.ts);
                queued_timeout_seconds = event
                    .payload_json
                    .get("timeout_seconds")
                    .and_then(Value::as_u64);
            }
            "delegate_started" => {
                started_at = Some(event.ts);
                started_timeout_seconds = event
                    .payload_json
                    .get("timeout_seconds")
                    .and_then(Value::as_u64);
            }
            _ => {}
        }
    }

    if session.parent_session_id.is_none() && queued_at.is_none() && started_at.is_none() {
        return None;
    }

    let phase = match session.state {
        SessionState::Ready => "queued",
        SessionState::Running => "running",
        SessionState::Completed => "completed",
        SessionState::Failed => "failed",
        SessionState::TimedOut => "timed_out",
    };
    let timeout_seconds = started_timeout_seconds.or(queued_timeout_seconds);
    let mode = if queued_at.is_some() || matches!(session.state, SessionState::Ready) {
        "async"
    } else {
        "inline"
    };
    let staleness = match session.state {
        SessionState::Ready => {
            session_delegate_staleness_at("queued", queued_at, timeout_seconds, now_ts)
        }
        SessionState::Running => session_delegate_staleness_at(
            if started_at.is_some() {
                "started"
            } else {
                "queued"
            },
            started_at.or(queued_at),
            timeout_seconds,
            now_ts,
        ),
        SessionState::Completed | SessionState::Failed | SessionState::TimedOut => None,
    };

    Some(SessionDelegateLifecycleRecord {
        mode,
        phase,
        queued_at,
        started_at,
        timeout_seconds,
        staleness,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_delegate_staleness_at(
    reference: &'static str,
    reference_at: Option<i64>,
    timeout_seconds: Option<u64>,
    now_ts: i64,
) -> Option<SessionDelegateStalenessRecord> {
    let reference_at = reference_at?;
    let threshold_seconds = timeout_seconds?;
    let elapsed_seconds = now_ts.saturating_sub(reference_at).max(0) as u64;
    let deadline_at = reference_at.saturating_add(threshold_seconds.min(i64::MAX as u64) as i64);
    let state = if elapsed_seconds > threshold_seconds {
        "overdue"
    } else {
        "fresh"
    };

    Some(SessionDelegateStalenessRecord {
        state,
        reference,
        elapsed_seconds,
        threshold_seconds,
        deadline_at,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_delegate_lifecycle_json(lifecycle: SessionDelegateLifecycleRecord) -> Value {
    json!({
        "mode": lifecycle.mode,
        "phase": lifecycle.phase,
        "queued_at": lifecycle.queued_at,
        "started_at": lifecycle.started_at,
        "timeout_seconds": lifecycle.timeout_seconds,
        "staleness": lifecycle.staleness.map(session_delegate_staleness_json),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_delegate_staleness_json(staleness: SessionDelegateStalenessRecord) -> Value {
    json!({
        "state": staleness.state,
        "reference": staleness.reference,
        "elapsed_seconds": staleness.elapsed_seconds,
        "threshold_seconds": staleness.threshold_seconds,
        "deadline_at": staleness.deadline_at,
    })
}

#[cfg(feature = "memory-sqlite")]
fn current_unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(feature = "memory-sqlite")]
fn ensure_visible(
    repo: &SessionRepository,
    current_session_id: &str,
    target_session_id: &str,
    visibility: SessionVisibility,
) -> Result<(), String> {
    let is_visible = match visibility {
        SessionVisibility::SelfOnly => current_session_id == target_session_id,
        SessionVisibility::Children => {
            repo.is_session_visible(current_session_id, target_session_id)?
        }
    };
    if is_visible {
        return Ok(());
    }
    Err(format!(
        "visibility_denied: session `{target_session_id}` is not visible from `{current_session_id}`"
    ))
}

fn required_payload_string(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("session tool requires payload.{field}"))
}

#[cfg(feature = "memory-sqlite")]
fn normalize_required_session_id(session_id: &str) -> Result<String, String> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err("session tool requires payload.session_id".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn optional_payload_limit(payload: &Value, field: &str, default: usize, max: usize) -> usize {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, max as u64) as usize)
        .unwrap_or(default)
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json(session: SessionSummaryRecord) -> Value {
    json!({
        "session_id": session.session_id,
        "kind": session.kind.as_str(),
        "parent_session_id": session.parent_session_id,
        "label": session.label,
        "state": session.state.as_str(),
        "created_at": session.created_at,
        "updated_at": session.updated_at,
        "turn_count": session.turn_count,
        "last_turn_at": session.last_turn_at,
        "last_error": session.last_error,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn session_event_json(event: SessionEventRecord) -> Value {
    json!({
        "id": event.id,
        "session_id": event.session_id,
        "event_kind": event.event_kind,
        "actor_session_id": event.actor_session_id,
        "payload_json": event.payload_json,
        "ts": event.ts,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_terminal_outcome_json(
    outcome: crate::session::repository::SessionTerminalOutcomeRecord,
) -> Value {
    json!({
        "session_id": outcome.session_id,
        "status": outcome.status,
        "payload": outcome.payload_json,
        "recorded_at": outcome.recorded_at,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use loongclaw_contracts::ToolCoreRequest;
    use rusqlite::params;
    use serde_json::{json, Value};

    use crate::config::{SessionVisibility, ToolConfig};
    use crate::memory::append_turn_direct;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        NewSessionEvent, NewSessionRecord, SessionEventRecord, SessionKind, SessionRepository,
        SessionState, SessionSummaryRecord,
    };

    use super::{execute_session_tool_with_config, execute_session_tool_with_policies};

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-session-tools-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
        }
    }

    fn overwrite_session_event_ts(
        config: &MemoryRuntimeConfig,
        session_id: &str,
        event_kind: &str,
        ts: i64,
    ) {
        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path for session tools test");
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
        let updated = conn
            .execute(
                "UPDATE session_events
                 SET ts = ?3
                 WHERE session_id = ?1 AND event_kind = ?2",
                params![session_id, event_kind, ts],
            )
            .expect("update session event ts");
        assert!(updated > 0, "expected at least one updated event row");
    }

    #[test]
    fn sessions_list_returns_current_session_and_children() {
        let config = isolated_memory_config("sessions-list");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "other-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Other".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create other");

        append_turn_direct("root-session", "user", "root turn", &config).expect("append root turn");
        append_turn_direct("child-session", "assistant", "child turn", &config)
            .expect("append child turn");
        append_turn_direct("other-session", "user", "other turn", &config)
            .expect("append other turn");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
        )
        .expect("sessions_list outcome");

        let sessions = outcome.payload["sessions"]
            .as_array()
            .expect("sessions array");
        let ids: Vec<&str> = sessions
            .iter()
            .filter_map(|item: &Value| item.get("session_id"))
            .filter_map(Value::as_str)
            .collect();
        assert!(ids.contains(&"root-session"));
        assert!(ids.contains(&"child-session"));
        assert!(!ids.contains(&"other-session"));
    }

    #[test]
    fn sessions_list_respects_self_visibility_policy() {
        let config = isolated_memory_config("sessions-list-self-only");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        let mut tool_config = ToolConfig::default();
        tool_config.sessions.visibility = SessionVisibility::SelfOnly;

        let outcome = execute_session_tool_with_policies(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
            &tool_config,
        )
        .expect("sessions_list outcome");

        let sessions = outcome.payload["sessions"]
            .as_array()
            .expect("sessions array");
        let ids: Vec<&str> = sessions
            .iter()
            .filter_map(|item: &Value| item.get("session_id"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(ids, vec!["root-session"]);
    }

    #[test]
    fn sessions_history_returns_transcript_without_control_events() {
        let config = isolated_memory_config("sessions-history");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({"status": "ok"}),
        })
        .expect("append event");

        append_turn_direct("child-session", "user", "hello", &config).expect("append user turn");
        append_turn_direct("child-session", "assistant", "world", &config)
            .expect("append assistant turn");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_history".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("sessions_history outcome");

        let turns = outcome.payload["turns"].as_array().expect("turns array");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0]["role"], "user");
        assert_eq!(turns[0]["content"], "hello");
        assert_eq!(turns[1]["role"], "assistant");
        assert_eq!(turns[1]["content"], "world");
    }

    #[test]
    fn session_status_returns_state_and_last_error() {
        let config = isolated_memory_config("session-status");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Failed,
        })
        .expect("create child");
        repo.update_session_state(
            "child-session",
            SessionState::Failed,
            Some("delegate_timeout".to_owned()),
        )
        .expect("update child status");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({"error": "delegate_timeout"}),
        })
        .expect("append event");
        repo.upsert_terminal_outcome(
            "child-session",
            "error",
            json!({
                "child_session_id": "child-session",
                "error": "delegate_timeout",
                "duration_ms": 12
            }),
        )
        .expect("upsert terminal outcome");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(outcome.payload["session"]["session_id"], "child-session");
        assert_eq!(outcome.payload["session"]["state"], "failed");
        assert_eq!(outcome.payload["session"]["last_error"], "delegate_timeout");
        assert_eq!(outcome.payload["terminal_outcome_state"], "present");
        assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
        assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
        assert_eq!(
            outcome.payload["terminal_outcome"]["payload"]["error"],
            "delegate_timeout"
        );
        let recent_events = outcome.payload["recent_events"]
            .as_array()
            .expect("recent_events array");
        assert_eq!(recent_events.len(), 1);
        assert_eq!(recent_events[0]["event_kind"], "delegate_failed");
    }

    #[test]
    fn session_status_reports_missing_terminal_outcome_for_recovered_failed_session() {
        let config = isolated_memory_config("session-status-recovered-failed");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Failed,
        })
        .expect("create child");
        repo.update_session_state(
            "child-session",
            SessionState::Failed,
            Some("opaque_recovery_failure".to_owned()),
        )
        .expect("update child status");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_recovery_applied".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "recovery_kind": "terminal_finalize_persist_failed",
                "recovered_state": "failed",
                "recovery_error": "delegate_terminal_finalize_failed: database busy",
                "attempted_terminal_event_kind": "delegate_completed",
                "attempted_outcome_status": "ok"
            }),
        })
        .expect("append event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(outcome.payload["session"]["session_id"], "child-session");
        assert_eq!(outcome.payload["session"]["state"], "failed");
        assert_eq!(
            outcome.payload["session"]["last_error"],
            "opaque_recovery_failure"
        );
        assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
        assert_eq!(
            outcome.payload["terminal_outcome_missing_reason"],
            "terminal_finalize_persist_failed"
        );
        assert_eq!(
            outcome.payload["recovery"]["kind"],
            "terminal_finalize_persist_failed"
        );
        assert_eq!(
            outcome.payload["recovery"]["event_kind"],
            "delegate_recovery_applied"
        );
        assert_eq!(
            outcome.payload["recovery"]["recovery_error"],
            "delegate_terminal_finalize_failed: database busy"
        );
        assert_eq!(
            outcome.payload["recovery"]["attempted_terminal_event_kind"],
            "delegate_completed"
        );
        assert_eq!(outcome.payload["recovery"]["source"], "event");
        assert!(outcome.payload["terminal_outcome"].is_null());
    }

    #[test]
    fn session_status_synthesizes_recovery_from_last_error_when_event_missing() {
        let config = isolated_memory_config("session-status-recovery-fallback");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Failed,
        })
        .expect("create child");
        repo.update_session_state(
            "child-session",
            SessionState::Failed,
            Some("delegate_terminal_finalize_failed: database busy".to_owned()),
        )
        .expect("update child status");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
        assert_eq!(
            outcome.payload["terminal_outcome_missing_reason"],
            "terminal_finalize_persist_failed"
        );
        assert_eq!(
            outcome.payload["recovery"]["kind"],
            "terminal_finalize_persist_failed"
        );
        assert_eq!(outcome.payload["recovery"]["source"], "last_error");
        assert_eq!(
            outcome.payload["recovery"]["recovery_error"],
            "delegate_terminal_finalize_failed: database busy"
        );
        assert!(outcome.payload["recovery"]["event_kind"].is_null());
    }

    #[test]
    fn session_status_synthesizes_unknown_recovery_when_metadata_missing() {
        let config = isolated_memory_config("session-status-recovery-unknown");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Failed,
        })
        .expect("create child");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
        assert_eq!(
            outcome.payload["terminal_outcome_missing_reason"],
            "unknown"
        );
        assert_eq!(outcome.payload["recovery"]["kind"], "unknown");
        assert_eq!(outcome.payload["recovery"]["source"], "none");
        assert!(outcome.payload["recovery"]["recovery_error"].is_null());
        assert!(outcome.payload["recovery"]["event_kind"].is_null());
    }

    #[test]
    fn session_recover_marks_overdue_queued_async_child_failed() {
        let config = isolated_memory_config("session-recover-overdue");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "label": "Child",
                "timeout_seconds": 30
            }),
        })
        .expect("append queued event");
        overwrite_session_event_ts(
            &config,
            "child-session",
            "delegate_queued",
            super::current_unix_ts() - 90,
        );

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_recover outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["session"]["state"], "failed");
        assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "failed");
        assert!(outcome.payload["delegate_lifecycle"]["staleness"].is_null());
        assert_eq!(outcome.payload["terminal_outcome_state"], "present");
        assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
        assert_eq!(
            outcome.payload["recovery_action"]["kind"],
            "queued_async_overdue_marked_failed"
        );
        assert_eq!(
            outcome.payload["recent_events"]
                .as_array()
                .expect("recent events array")
                .last()
                .expect("latest recent event")["event_kind"],
            "delegate_recovery_applied"
        );
    }

    #[test]
    fn session_recover_rejects_fresh_queued_child() {
        let config = isolated_memory_config("session-recover-fresh");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 60
            }),
        })
        .expect("append queued event");

        let error = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect_err("fresh queued child should be rejected");

        assert!(
            error.contains("session_recover_not_recoverable"),
            "expected recoverability rejection, got: {error}"
        );
    }

    #[test]
    fn session_recover_rejects_running_child() {
        let config = isolated_memory_config("session-recover-running");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 30
            }),
        })
        .expect("append queued event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 30
            }),
        })
        .expect("append started event");

        let error = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect_err("running child should be rejected");

        assert!(
            error.contains("session_recover_not_recoverable"),
            "expected recoverability rejection, got: {error}"
        );
    }

    #[test]
    fn session_delegate_lifecycle_marks_overdue_queued_child() {
        let session = SessionSummaryRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
            created_at: 100,
            updated_at: 100,
            turn_count: 0,
            last_turn_at: None,
            last_error: None,
        };
        let events = vec![SessionEventRecord {
            id: 1,
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 30
            }),
            ts: 100,
        }];

        let lifecycle = super::session_delegate_lifecycle_at(&session, &events, 140)
            .expect("delegate lifecycle");

        assert_eq!(lifecycle.mode, "async");
        assert_eq!(lifecycle.phase, "queued");
        assert_eq!(lifecycle.queued_at, Some(100));
        assert_eq!(lifecycle.started_at, None);
        assert_eq!(lifecycle.timeout_seconds, Some(30));
        let staleness = lifecycle.staleness.expect("staleness");
        assert_eq!(staleness.state, "overdue");
        assert_eq!(staleness.reference, "queued");
        assert_eq!(staleness.elapsed_seconds, 40);
        assert_eq!(staleness.threshold_seconds, 30);
        assert_eq!(staleness.deadline_at, 130);
    }

    #[test]
    fn session_status_includes_delegate_lifecycle_for_queued_child() {
        let config = isolated_memory_config("session-status-delegate-lifecycle");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "label": "Child",
                "timeout_seconds": 60
            }),
        })
        .expect("append queued event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(outcome.payload["delegate_lifecycle"]["mode"], "async");
        assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "queued");
        assert_eq!(outcome.payload["delegate_lifecycle"]["timeout_seconds"], 60);
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["staleness"]["reference"],
            "queued"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["staleness"]["state"],
            "fresh"
        );
        assert!(outcome.payload["delegate_lifecycle"]["queued_at"].is_number());
        assert!(outcome.payload["delegate_lifecycle"]["started_at"].is_null());
    }

    #[test]
    fn session_tools_reject_invisible_sessions() {
        let config = isolated_memory_config("session-visibility");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "other-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Other".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create other");

        let error = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "other-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect_err("invisible session should be rejected");

        assert!(
            error.contains("visibility_denied"),
            "expected visibility_denied, got: {error}"
        );
    }

    #[test]
    fn session_status_returns_inferred_legacy_current_session_without_backfill() {
        let config = isolated_memory_config("legacy-session-status");
        append_turn_direct("delegate:legacy-child", "user", "hello", &config)
            .expect("append user turn");
        append_turn_direct("delegate:legacy-child", "assistant", "done", &config)
            .expect("append assistant turn");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "delegate:legacy-child"
                }),
            },
            "delegate:legacy-child",
            &config,
        )
        .expect("legacy session_status outcome");

        assert_eq!(
            outcome.payload["session"]["session_id"],
            "delegate:legacy-child"
        );
        assert_eq!(outcome.payload["session"]["kind"], "delegate_child");
        assert_eq!(outcome.payload["session"]["state"], "ready");
        assert_eq!(outcome.payload["terminal_outcome_state"], "not_terminal");
        assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
        assert!(outcome.payload["delegate_lifecycle"].is_null());
        assert!(outcome.payload["terminal_outcome"].is_null());
        assert_eq!(
            outcome.payload["recent_events"]
                .as_array()
                .expect("recent_events array")
                .len(),
            0
        );

        let repo = SessionRepository::new(&config).expect("repository");
        assert!(repo
            .load_session("delegate:legacy-child")
            .expect("load legacy session")
            .is_none());
    }

    #[test]
    fn session_status_allows_visible_descendant_delegate_session() {
        let config = isolated_memory_config("descendant-session-status");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create child");
        repo.create_session(NewSessionRecord {
            session_id: "grandchild-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-session".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create grandchild");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "grandchild-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("descendant session_status outcome");

        assert_eq!(
            outcome.payload["session"]["session_id"],
            "grandchild-session"
        );
        assert_eq!(outcome.payload["session"]["kind"], "delegate_child");
    }

    #[test]
    fn session_events_returns_ordered_tail_and_respects_after_id() {
        let config = isolated_memory_config("session-events");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        let first = repo
            .append_event(NewSessionEvent {
                session_id: "child-session".to_owned(),
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({"step": 1}),
            })
            .expect("append first event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_progress".to_owned(),
            actor_session_id: Some("child-session".to_owned()),
            payload_json: json!({"step": 2}),
        })
        .expect("append second event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({"step": 3}),
        })
        .expect("append third event");

        let full = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_events".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_events outcome");
        let full_events = full.payload["events"].as_array().expect("events array");
        assert_eq!(full_events.len(), 3);
        assert_eq!(full_events[0]["event_kind"], "delegate_started");
        assert_eq!(full_events[1]["event_kind"], "delegate_progress");
        assert_eq!(full_events[2]["event_kind"], "delegate_completed");

        let incremental = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_events".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "after_id": first.id,
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("incremental session_events outcome");
        let incremental_events = incremental.payload["events"]
            .as_array()
            .expect("incremental events array");
        assert_eq!(incremental_events.len(), 2);
        assert_eq!(incremental_events[0]["event_kind"], "delegate_progress");
        assert_eq!(incremental_events[1]["event_kind"], "delegate_completed");
    }
}
