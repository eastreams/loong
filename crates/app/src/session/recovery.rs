use serde_json::{json, Value};

use crate::session::repository::{FinalizeSessionTerminalRequest, SessionEventRecord};

pub(crate) const RECOVERY_EVENT_KIND: &str = "delegate_recovery_applied";
pub(crate) const RECOVERY_SOURCE_EVENT: &str = "event";
pub(crate) const RECOVERY_SOURCE_LAST_ERROR: &str = "last_error";
pub(crate) const RECOVERY_SOURCE_NONE: &str = "none";

pub(crate) const RECOVERY_KIND_UNKNOWN: &str = "unknown";
pub(crate) const RECOVERY_KIND_TERMINAL_FINALIZE_PERSIST_FAILED: &str =
    "terminal_finalize_persist_failed";
pub(crate) const RECOVERY_KIND_ASYNC_SPAWN_FAILURE_PERSIST_FAILED: &str =
    "async_spawn_failure_persist_failed";
pub(crate) const RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED: &str =
    "queued_async_overdue_marked_failed";
pub(crate) const RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED: &str =
    "running_async_overdue_marked_failed";

const TERMINAL_FINALIZE_LAST_ERROR_PREFIX: &str = "delegate_terminal_finalize_failed:";
const ASYNC_SPAWN_PERSIST_LAST_ERROR_PREFIX: &str = "delegate_async_spawn_failure_persist_failed:";
const QUEUED_ASYNC_OVERDUE_LAST_ERROR_PREFIX: &str = "delegate_async_queued_overdue_marked_failed:";
const RUNNING_ASYNC_OVERDUE_LAST_ERROR_PREFIX: &str =
    "delegate_async_running_overdue_marked_failed:";

const RECOVERY_KIND_FIELD: &str = "recovery_kind";
const RECOVERED_STATE_FIELD: &str = "recovered_state";
const RECOVERY_ERROR_FIELD: &str = "recovery_error";
const ORIGINAL_ERROR_FIELD: &str = "original_error";
const ATTEMPTED_TERMINAL_EVENT_KIND_FIELD: &str = "attempted_terminal_event_kind";
const ATTEMPTED_OUTCOME_STATUS_FIELD: &str = "attempted_outcome_status";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionRecoveryRecord {
    pub source: String,
    pub kind: String,
    pub event_kind: String,
    pub recovered_state: Option<String>,
    pub recovery_error: Option<String>,
    pub original_error: Option<String>,
    pub attempted_terminal_event_kind: Option<String>,
    pub attempted_outcome_status: Option<String>,
    pub ts: i64,
}

pub(crate) fn build_terminal_finalize_recovery_payload(
    request: &FinalizeSessionTerminalRequest,
    recovery_error: &str,
) -> Value {
    json!({
        RECOVERY_KIND_FIELD: RECOVERY_KIND_TERMINAL_FINALIZE_PERSIST_FAILED,
        RECOVERED_STATE_FIELD: "failed",
        RECOVERY_ERROR_FIELD: recovery_error,
        ATTEMPTED_TERMINAL_EVENT_KIND_FIELD: request.event_kind,
        ATTEMPTED_OUTCOME_STATUS_FIELD: request.outcome_status,
        "attempted_last_error": request.last_error,
    })
}

pub(crate) fn build_async_spawn_failure_recovery_payload(
    label: Option<&str>,
    original_error: &str,
    recovery_error: &str,
) -> Value {
    json!({
        RECOVERY_KIND_FIELD: RECOVERY_KIND_ASYNC_SPAWN_FAILURE_PERSIST_FAILED,
        RECOVERED_STATE_FIELD: "failed",
        RECOVERY_ERROR_FIELD: recovery_error,
        ORIGINAL_ERROR_FIELD: original_error,
        "label": label,
    })
}

pub(crate) fn build_queued_async_overdue_recovery_payload(
    label: Option<&str>,
    queued_at: i64,
    elapsed_seconds: u64,
    timeout_seconds: u64,
    deadline_at: i64,
    recovery_error: &str,
) -> Value {
    json!({
        RECOVERY_KIND_FIELD: RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
        RECOVERED_STATE_FIELD: "failed",
        RECOVERY_ERROR_FIELD: recovery_error,
        "label": label,
        "queued_at": queued_at,
        "elapsed_seconds": elapsed_seconds,
        "timeout_seconds": timeout_seconds,
        "deadline_at": deadline_at,
        "reference": "queued",
    })
}

pub(crate) fn build_running_async_overdue_recovery_payload(
    label: Option<&str>,
    queued_at: Option<i64>,
    started_at: Option<i64>,
    reference: &str,
    elapsed_seconds: u64,
    timeout_seconds: u64,
    deadline_at: i64,
    recovery_error: &str,
) -> Value {
    json!({
        RECOVERY_KIND_FIELD: RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED,
        RECOVERED_STATE_FIELD: "failed",
        RECOVERY_ERROR_FIELD: recovery_error,
        "label": label,
        "queued_at": queued_at,
        "started_at": started_at,
        "elapsed_seconds": elapsed_seconds,
        "timeout_seconds": timeout_seconds,
        "deadline_at": deadline_at,
        "reference": reference,
    })
}

pub(crate) fn observe_missing_recovery(
    recent_events: &[SessionEventRecord],
    last_error: Option<&str>,
) -> SessionRecoveryRecord {
    recent_events
        .iter()
        .rev()
        .find_map(parse_recovery_event)
        .unwrap_or_else(|| synthesize_recovery_from_last_error(last_error))
}

pub(crate) fn recovery_json(recovery: SessionRecoveryRecord) -> Value {
    json!({
        "source": recovery.source,
        "kind": recovery.kind,
        "event_kind": if recovery.event_kind.is_empty() {
            Value::Null
        } else {
            Value::String(recovery.event_kind)
        },
        "recovered_state": recovery.recovered_state,
        "recovery_error": recovery.recovery_error,
        "original_error": recovery.original_error,
        "attempted_terminal_event_kind": recovery.attempted_terminal_event_kind,
        "attempted_outcome_status": recovery.attempted_outcome_status,
        "ts": if recovery.ts == 0 { Value::Null } else { Value::from(recovery.ts) },
    })
}

fn parse_recovery_event(event: &SessionEventRecord) -> Option<SessionRecoveryRecord> {
    if event.event_kind != RECOVERY_EVENT_KIND {
        return None;
    }
    let kind = event
        .payload_json
        .get(RECOVERY_KIND_FIELD)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    Some(SessionRecoveryRecord {
        source: RECOVERY_SOURCE_EVENT.to_owned(),
        kind,
        event_kind: event.event_kind.clone(),
        recovered_state: event
            .payload_json
            .get(RECOVERED_STATE_FIELD)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        recovery_error: event
            .payload_json
            .get(RECOVERY_ERROR_FIELD)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        original_error: event
            .payload_json
            .get(ORIGINAL_ERROR_FIELD)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        attempted_terminal_event_kind: event
            .payload_json
            .get(ATTEMPTED_TERMINAL_EVENT_KIND_FIELD)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        attempted_outcome_status: event
            .payload_json
            .get(ATTEMPTED_OUTCOME_STATUS_FIELD)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        ts: event.ts,
    })
}

fn synthesize_recovery_from_last_error(last_error: Option<&str>) -> SessionRecoveryRecord {
    let recovery_error = last_error
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    SessionRecoveryRecord {
        source: if recovery_error.is_some() {
            RECOVERY_SOURCE_LAST_ERROR.to_owned()
        } else {
            RECOVERY_SOURCE_NONE.to_owned()
        },
        kind: recovery_kind_from_last_error(recovery_error.as_deref()).to_owned(),
        event_kind: String::new(),
        recovered_state: None,
        recovery_error,
        original_error: None,
        attempted_terminal_event_kind: None,
        attempted_outcome_status: None,
        ts: 0,
    }
}

fn recovery_kind_from_last_error(last_error: Option<&str>) -> &'static str {
    match last_error {
        Some(last_error) if last_error.starts_with(TERMINAL_FINALIZE_LAST_ERROR_PREFIX) => {
            RECOVERY_KIND_TERMINAL_FINALIZE_PERSIST_FAILED
        }
        Some(last_error) if last_error.starts_with(ASYNC_SPAWN_PERSIST_LAST_ERROR_PREFIX) => {
            RECOVERY_KIND_ASYNC_SPAWN_FAILURE_PERSIST_FAILED
        }
        Some(last_error) if last_error.starts_with(QUEUED_ASYNC_OVERDUE_LAST_ERROR_PREFIX) => {
            RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED
        }
        Some(last_error) if last_error.starts_with(RUNNING_ASYNC_OVERDUE_LAST_ERROR_PREFIX) => {
            RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED
        }
        Some(_) | None => RECOVERY_KIND_UNKNOWN,
    }
}
