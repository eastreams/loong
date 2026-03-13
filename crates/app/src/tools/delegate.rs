use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::FutureExt;
use loongclaw_contracts::ToolCoreOutcome;
use serde_json::{json, Value};
use std::panic::AssertUnwindSafe;

#[cfg(test)]
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateRequest {
    pub task: String,
    pub label: Option<String>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub label: Option<String>,
    pub timeout_seconds: u64,
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(test)]
pub fn parse_delegate_request(payload: &Value) -> Result<DelegateRequest, String> {
    parse_delegate_request_with_default_timeout(payload, DEFAULT_TIMEOUT_SECONDS)
}

pub fn parse_delegate_request_with_default_timeout(
    payload: &Value,
    default_timeout_seconds: u64,
) -> Result<DelegateRequest, String> {
    let task = required_payload_string(payload, "task")?;
    let label = payload
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned);
    let timeout_seconds = payload
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(default_timeout_seconds);

    Ok(DelegateRequest {
        task,
        label,
        timeout_seconds,
    })
}

pub fn next_delegate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{now_ms:x}{counter:x}")
}

pub fn delegate_success_outcome(
    child_session_id: String,
    label: Option<String>,
    final_output: String,
    turn_count: usize,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "final_output": final_output,
            "turn_count": turn_count,
            "duration_ms": duration_ms,
        }),
    }
}

pub fn delegate_async_queued_outcome(
    child_session_id: String,
    label: Option<String>,
    timeout_seconds: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "mode": "async",
            "state": "queued",
            "timeout_seconds": timeout_seconds,
        }),
    }
}

pub fn delegate_timeout_outcome(
    child_session_id: String,
    label: Option<String>,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "timeout".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "duration_ms": duration_ms,
            "error": "delegate_timeout",
        }),
    }
}

pub fn delegate_error_outcome(
    child_session_id: String,
    label: Option<String>,
    error: String,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "error".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "duration_ms": duration_ms,
            "error": error,
        }),
    }
}

#[cfg(feature = "memory-sqlite")]
fn finalize_async_delegate_spawn_failure(
    memory_config: &crate::memory::runtime_config::MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    let outcome = delegate_error_outcome(child_session_id.to_owned(), label, error.clone(), 0);
    repo.finalize_session_terminal(
        child_session_id,
        crate::session::repository::FinalizeSessionTerminalRequest {
            state: crate::session::repository::SessionState::Failed,
            last_error: Some(error.clone()),
            event_kind: "delegate_spawn_failed".to_owned(),
            actor_session_id: Some(parent_session_id.to_owned()),
            event_payload_json: json!({
                "error": error,
            }),
            outcome_status: outcome.status,
            outcome_payload_json: outcome.payload,
        },
    )?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &crate::memory::runtime_config::MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    error: String,
) -> Result<(), String> {
    let recovery_label = label.clone();
    match finalize_async_delegate_spawn_failure(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        error.clone(),
    ) {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let repo = crate::session::repository::SessionRepository::new(memory_config)?;
            let recovery_error = format!(
                "delegate_async_spawn_failure_persist_failed: {finalize_error}; original spawn error: {error}"
            );
            match repo.transition_session_with_event_if_current(
                child_session_id,
                crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state: crate::session::repository::SessionState::Ready,
                    next_state: crate::session::repository::SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: crate::session::recovery::RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
                    event_payload_json:
                        crate::session::recovery::build_async_spawn_failure_recovery_payload(
                            recovery_label.as_deref(),
                            &error,
                            &recovery_error,
                        ),
                },
            ) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => {
                    let current_state = repo
                        .load_session(child_session_id)?
                        .map(|session| session.state.as_str().to_owned())
                        .unwrap_or_else(|| "missing".to_owned());
                    Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                    ))
                }
                Err(recovery_event_error) => match repo.update_session_state_if_current(
                    child_session_id,
                    crate::session::repository::SessionState::Ready,
                    crate::session::repository::SessionState::Failed,
                    Some(recovery_error.clone()),
                ) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => {
                        let current_state = repo
                            .load_session(child_session_id)?
                            .map(|session| session.state.as_str().to_owned())
                            .unwrap_or_else(|| "missing".to_owned());
                        Err(format!(
                            "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                        ))
                    }
                    Err(mark_error) => Err(format!(
                        "{recovery_error}; delegate_async_spawn_recovery_failed: {mark_error}; delegate_async_spawn_recovery_event_failed: {recovery_event_error}"
                    )),
                },
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn format_async_delegate_spawn_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_async_spawn_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };
    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_async_spawn_panic: {}", *message),
        Err(_) => "delegate_async_spawn_panic".to_owned(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn spawn_async_delegate_detached(
    runtime_handle: tokio::runtime::Handle,
    memory_config: crate::memory::runtime_config::MemoryRuntimeConfig,
    spawner: Arc<dyn AsyncDelegateSpawner>,
    request: AsyncDelegateSpawnRequest,
) {
    let child_session_id = request.child_session_id.clone();
    let parent_session_id = request.parent_session_id.clone();
    let label = request.label.clone();
    runtime_handle.spawn(async move {
        let spawn_failure = match AssertUnwindSafe(spawner.spawn(request)).catch_unwind().await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(panic_payload) => Some(format_async_delegate_spawn_panic(panic_payload)),
        };
        if let Some(error) = spawn_failure {
            if let Err(finalize_error) = finalize_async_delegate_spawn_failure_with_recovery(
                &memory_config,
                &child_session_id,
                &parent_session_id,
                label,
                error.clone(),
            ) {
                eprintln!(
                    "error: async delegate spawn failure persistence failed for `{child_session_id}`: {finalize_error}; original spawn error: {error}"
                );
            }
        }
    });
}

#[cfg(feature = "memory-sqlite")]
pub async fn execute_delegate_async_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &crate::memory::runtime_config::MemoryRuntimeConfig,
    tool_config: &crate::config::ToolConfig,
    spawner: Arc<dyn AsyncDelegateSpawner>,
) -> Result<ToolCoreOutcome, String> {
    if !tool_config.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let delegate_request = parse_delegate_request_with_default_timeout(
        &payload,
        tool_config.delegate.timeout_seconds,
    )?;
    let runtime_handle = tokio::runtime::Handle::try_current()
        .map_err(|error| format!("delegate_async_runtime_unavailable: {error}"))?;
    let repo = crate::session::repository::SessionRepository::new(memory_config)?;
    let current_depth = repo.session_lineage_depth(current_session_id)?;
    let next_child_depth = current_depth.saturating_add(1);
    if next_child_depth > tool_config.delegate.max_depth {
        return Err(format!(
            "delegate_depth_exceeded: next child depth {next_child_depth} exceeds configured max_depth {}",
            tool_config.delegate.max_depth
        ));
    }

    let child_session_id = next_delegate_session_id();
    let child_label = delegate_request.label.clone();
    repo.create_session_with_event(crate::session::repository::CreateSessionWithEventRequest {
        session: crate::session::repository::NewSessionRecord {
            session_id: child_session_id.clone(),
            kind: crate::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some(current_session_id.to_owned()),
            label: child_label.clone(),
            state: crate::session::repository::SessionState::Ready,
        },
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some(current_session_id.to_owned()),
        event_payload_json: json!({
            "task": delegate_request.task.clone(),
            "label": child_label.clone(),
            "timeout_seconds": delegate_request.timeout_seconds,
        }),
    })?;

    let spawn_request = AsyncDelegateSpawnRequest {
        child_session_id: child_session_id.clone(),
        parent_session_id: current_session_id.to_owned(),
        task: delegate_request.task,
        label: child_label.clone(),
        timeout_seconds: delegate_request.timeout_seconds,
    };

    spawn_async_delegate_detached(
        runtime_handle,
        memory_config.clone(),
        Arc::clone(&spawner),
        spawn_request,
    );

    Ok(delegate_async_queued_outcome(
        child_session_id,
        delegate_request.label,
        delegate_request.timeout_seconds,
    ))
}

fn required_payload_string(payload: &Value, field: &str) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("delegate tool requires payload.{field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_delegate_request_requires_task() {
        let error =
            parse_delegate_request(&json!({})).expect_err("missing task should be rejected");
        assert!(error.contains("payload.task"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_uses_defaults() {
        let request = parse_delegate_request(&json!({
            "task": "research"
        }))
        .expect("delegate request");
        assert_eq!(request.task, "research");
        assert_eq!(request.label, None);
        assert_eq!(request.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn delegate_session_ids_use_expected_prefix() {
        let session_id = next_delegate_session_id();
        assert!(session_id.starts_with("delegate:"));
    }
}
