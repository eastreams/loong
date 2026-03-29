#[cfg(feature = "memory-sqlite")]
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "memory-sqlite")]
use tokio::time::{Duration, Instant, sleep};

use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

use super::payload::{optional_payload_limit, optional_payload_string, required_payload_string};

use crate::config::{SessionVisibility, ToolConfig};
#[cfg(feature = "memory-sqlite")]
use crate::conversation::ConstrainedSubagentExecution;
use crate::memory;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{
    RECOVERY_EVENT_KIND, RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
    RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED, SessionRecoveryRecord,
    build_queued_async_overdue_recovery_payload, build_running_async_overdue_recovery_payload,
    observe_missing_recovery, recovery_json,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::{
    DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED, DELEGATE_CANCEL_REQUESTED_EVENT_KIND,
    DELEGATE_CANCELLED_EVENT_KIND, delegate_cancelled_error,
};
#[cfg(feature = "memory-sqlite")]
use crate::tools::ToolView;
#[cfg(feature = "memory-sqlite")]
use crate::tools::runtime_config::ToolRuntimeNarrowing;

#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewSessionRecord, NewSessionToolPolicyRecord, SessionEventRecord, SessionKind,
    SessionObservationRecord, SessionRepository, SessionState, SessionSummaryRecord,
    SessionTerminalOutcomeRecord, SessionToolPolicyRecord,
};

#[cfg(feature = "memory-sqlite")]
fn delegate_error_outcome(
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
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionInspectionSnapshot {
    pub session: SessionSummaryRecord,
    pub terminal_outcome: Option<SessionTerminalOutcomeRecord>,
    pub recent_events: Vec<SessionEventRecord>,
    pub delegate_events: Vec<SessionEventRecord>,
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
    execution: Option<ConstrainedSubagentExecution>,
    staleness: Option<SessionDelegateStalenessRecord>,
    cancellation: Option<SessionDelegateCancellationRecord>,
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
struct SessionDelegateCancellationRecord {
    state: &'static str,
    reference: String,
    requested_at: i64,
    reason: String,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionsListRequest {
    limit: usize,
    state: Option<SessionState>,
    kind: Option<SessionKind>,
    parent_session_id: Option<String>,
    overdue_only: bool,
    include_archived: bool,
    include_delegate_lifecycle: bool,
}

#[cfg(feature = "memory-sqlite")]
impl SessionsListRequest {
    fn effective_include_delegate_lifecycle(&self) -> bool {
        self.include_delegate_lifecycle || self.overdue_only
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionTargetRequest {
    session_ids: Vec<String>,
    legacy_single: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionMutationRequest {
    target: SessionTargetRequest,
    dry_run: bool,
}

#[cfg(feature = "memory-sqlite")]
impl SessionMutationRequest {
    fn use_legacy_single_response(&self) -> bool {
        self.target.legacy_single && !self.dry_run
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionRecoverPlan {
    expected_state: SessionState,
    recovery_kind: &'static str,
    reference: &'static str,
    queued_at: Option<i64>,
    started_at: Option<i64>,
    elapsed_seconds: u64,
    timeout_seconds: u64,
    deadline_at: i64,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionCancelPlan {
    Queued,
    Running,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionArchivePlan {
    expected_state: SessionState,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionToolPolicySetRequest {
    session_id: String,
    tool_ids: Option<Vec<String>>,
    runtime_narrowing: Option<ToolRuntimeNarrowing>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
struct SessionToolActionOutcome {
    inspection: Value,
    action: Value,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
struct SessionBatchResultRecord {
    session_id: String,
    result: &'static str,
    message: Option<String>,
    action: Option<Value>,
    inspection: Option<Value>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
struct SessionWaitTargetState {
    index: usize,
    session_id: String,
    next_after_id: i64,
    observed_events: Vec<SessionEventRecord>,
    latest_inspection: Option<SessionInspectionSnapshot>,
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn set_session_batch_result(
    results: &mut [Option<SessionBatchResultRecord>],
    index: usize,
    result: SessionBatchResultRecord,
) -> Result<(), String> {
    let Some(slot) = results.get_mut(index) else {
        return Err(format!(
            "session_wait_internal_error: result slot `{index}` is out of bounds"
        ));
    };
    *slot = Some(result);
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn collect_session_batch_results(
    results: Vec<Option<SessionBatchResultRecord>>,
) -> Result<Vec<SessionBatchResultRecord>, String> {
    let mut collected = Vec::with_capacity(results.len());
    for (index, result) in results.into_iter().enumerate() {
        let Some(result) = result else {
            return Err(format!(
                "session_wait_internal_error: missing batch result at index `{index}`"
            ));
        };
        collected.push(result);
    }
    Ok(collected)
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
        let ToolCoreRequest { tool_name, payload } = request;
        let tool_catalog = super::tool_catalog();
        let tool_descriptor = tool_catalog.resolve(tool_name.as_str());
        let visibility_gate = tool_descriptor.map(|descriptor| descriptor.visibility_gate);
        let mutation_gate = super::catalog::ToolVisibilityGate::SessionMutation;
        let uses_mutation_gate = visibility_gate == Some(mutation_gate);
        let mutation_disabled = !tool_config.sessions.allow_mutation;

        if uses_mutation_gate && mutation_disabled {
            return Err(format!(
                "app_tool_disabled: session mutation tool `{tool_name}` is disabled by config"
            ));
        }

        match tool_name.as_str() {
            "sessions_list" => {
                execute_sessions_list(payload, current_session_id, config, tool_config)
            }
            "session_events" => {
                execute_session_events(payload, current_session_id, config, tool_config)
            }
            "sessions_history" => {
                execute_sessions_history(payload, current_session_id, config, tool_config)
            }
            "session_tool_policy_status" => {
                execute_session_tool_policy_status(payload, current_session_id, config, tool_config)
            }
            "session_tool_policy_set" => {
                execute_session_tool_policy_set(payload, current_session_id, config, tool_config)
            }
            "session_tool_policy_clear" => {
                execute_session_tool_policy_clear(payload, current_session_id, config, tool_config)
            }
            "session_status" => {
                execute_session_status(payload, current_session_id, config, tool_config)
            }
            "session_cancel" => {
                execute_session_cancel(payload, current_session_id, config, tool_config)
            }
            "session_archive" => {
                execute_session_archive(payload, current_session_id, config, tool_config)
            }
            "session_recover" => {
                execute_session_recover(payload, current_session_id, config, tool_config)
            }
            other => Err(format!(
                "app_tool_not_found: unknown session tool `{other}`"
            )),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
pub(super) async fn wait_for_session_tool_with_policies(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_target_request(&payload)?;
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(1_000)
        .clamp(1, 30_000);
    let event_limit = tool_config.sessions.history_limit.min(50);

    if request.legacy_single {
        let target_session_id = legacy_single_session_id(&request.session_ids)?;
        return wait_for_single_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            after_id,
            timeout_ms,
            event_limit,
        )
        .await;
    }

    wait_for_session_batch_with_policies(
        request.session_ids,
        current_session_id,
        config,
        tool_config,
        after_id,
        timeout_ms,
        event_limit,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
fn execute_sessions_list(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_sessions_list_request(&payload, tool_config)?;
    let include_delegate_lifecycle = request.effective_include_delegate_lifecycle();
    let now_ts = current_unix_ts();
    let mut sessions = repo.list_visible_sessions(current_session_id)?;
    if tool_config.sessions.visibility == SessionVisibility::SelfOnly {
        sessions.retain(|session| session.session_id == current_session_id);
    }
    if let Some(state) = request.state {
        sessions.retain(|session| session.state == state);
    }
    if let Some(kind) = request.kind {
        sessions.retain(|session| session.kind == kind);
    }
    if let Some(parent_session_id) = request.parent_session_id.as_deref() {
        sessions.retain(|session| session.parent_session_id.as_deref() == Some(parent_session_id));
    }
    if !request.include_archived {
        sessions.retain(|session| session.archived_at.is_none());
    }

    let mut listed_sessions = Vec::new();
    for session in sessions {
        let delegate_lifecycle = if include_delegate_lifecycle {
            let delegate_events = load_delegate_lifecycle_events(&repo, &session)?;
            session_delegate_lifecycle_at(&session, delegate_events.as_slice(), now_ts)
        } else {
            None
        };
        if request.overdue_only
            && !delegate_lifecycle
                .as_ref()
                .and_then(|lifecycle| lifecycle.staleness.as_ref())
                .map(|staleness| staleness.state == "overdue")
                .unwrap_or(false)
        {
            continue;
        }
        listed_sessions.push((session, delegate_lifecycle));
    }

    let matched_count = listed_sessions.len();
    listed_sessions.truncate(request.limit);
    let returned_count = listed_sessions.len();
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filters": sessions_list_filters_json(&request),
            "matched_count": matched_count,
            "returned_count": returned_count,
            "sessions": listed_sessions
                .into_iter()
                .map(|(session, delegate_lifecycle)| {
                    session_summary_json_with_delegate_lifecycle(
                        session,
                        delegate_lifecycle,
                        include_delegate_lifecycle,
                    )
                })
                .collect::<Vec<_>>(),
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
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
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
    let target_session_id = required_payload_string(&payload, "session_id", "session tool")?;
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
fn execute_session_tool_policy_status(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let target_session_id =
        resolve_session_tool_policy_target_session_id(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;
    let policy = build_session_tool_policy_status_payload(&repo, &target_session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_status",
            "current_session_id": current_session_id,
            "target_session_id": target_session_id,
            "policy": policy,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_tool_policy_set(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_session_tool_policy_set_request(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &request.session_id,
        tool_config.sessions.visibility,
    )?;
    ensure_policy_target_session_exists(&repo, &request.session_id, current_session_id)?;

    let existing_policy = repo.load_session_tool_policy(&request.session_id)?;
    let existing_tool_ids = existing_policy
        .as_ref()
        .map(|policy| policy.requested_tool_ids.clone())
        .unwrap_or_default();
    let existing_runtime_narrowing = existing_policy
        .as_ref()
        .map(|policy| policy.runtime_narrowing.clone())
        .unwrap_or_default();

    let next_tool_ids = match request.tool_ids {
        Some(tool_ids) => {
            resolve_session_tool_policy_tool_ids(&repo, &request.session_id, tool_config, tool_ids)?
        }
        None => existing_tool_ids,
    };
    let next_runtime_narrowing = request
        .runtime_narrowing
        .unwrap_or(existing_runtime_narrowing);
    let clears_policy = next_tool_ids.is_empty() && next_runtime_narrowing.is_empty();

    let action = if clears_policy {
        if existing_policy.is_some() {
            repo.delete_session_tool_policy(&request.session_id)?;
            "cleared"
        } else {
            "unchanged"
        }
    } else {
        let next_policy = NewSessionToolPolicyRecord {
            session_id: request.session_id.clone(),
            requested_tool_ids: next_tool_ids.clone(),
            runtime_narrowing: next_runtime_narrowing.clone(),
        };
        let unchanged = existing_policy
            .as_ref()
            .is_some_and(|policy| policy.requested_tool_ids == next_tool_ids)
            && existing_policy
                .as_ref()
                .is_some_and(|policy| policy.runtime_narrowing == next_runtime_narrowing);
        if unchanged {
            "unchanged"
        } else {
            repo.upsert_session_tool_policy(next_policy)?;
            if existing_policy.is_some() {
                "updated"
            } else {
                "created"
            }
        }
    };
    let policy = build_session_tool_policy_status_payload(&repo, &request.session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_set",
            "action": action,
            "current_session_id": current_session_id,
            "target_session_id": request.session_id,
            "policy": policy,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_tool_policy_clear(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let target_session_id =
        resolve_session_tool_policy_target_session_id(&payload, current_session_id)?;
    ensure_visible(
        &repo,
        current_session_id,
        &target_session_id,
        tool_config.sessions.visibility,
    )?;

    let cleared = repo.delete_session_tool_policy(&target_session_id)?;
    let policy = build_session_tool_policy_status_payload(&repo, &target_session_id, tool_config)?;

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "session_tool_policy_clear",
            "action": if cleared { "cleared" } else { "unchanged" },
            "current_session_id": current_session_id,
            "target_session_id": target_session_id,
            "policy": policy,
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
    let request = parse_session_target_request(&payload)?;
    if request.legacy_single {
        let target_session_id = legacy_single_session_id(&request.session_ids)?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            5,
        )?;

        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: session_inspection_payload(snapshot),
        });
    }

    let mut results = Vec::with_capacity(request.session_ids.len());
    for target_session_id in &request.session_ids {
        results.push(execute_session_status_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload_without_dry_run(
            "session_status",
            current_session_id,
            request.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_recover(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let recover_plan = build_session_recover_plan(&snapshot, current_unix_ts())?;
        let outcome = apply_session_recover_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            &recover_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("recovery_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_recover_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_recover",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_cancel(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let cancel_plan = build_session_cancel_plan(&snapshot)?;
        let outcome = apply_session_cancel_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            cancel_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("cancel_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_cancel_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_cancel",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_archive(
    payload: Value,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_session_mutation_request(&payload)?;
    if request.use_legacy_single_response() {
        let target_session_id = legacy_single_session_id(&request.target.session_ids)?;
        let repo = SessionRepository::new(config)?;
        ensure_visible(
            &repo,
            current_session_id,
            target_session_id,
            tool_config.sessions.visibility,
        )?;
        let snapshot = inspect_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            10,
        )?;
        let archive_plan = build_session_archive_plan(&snapshot)?;
        let outcome = apply_session_archive_plan(
            &repo,
            target_session_id,
            current_session_id,
            config,
            tool_config,
            &snapshot,
            &archive_plan,
        )?;
        let mut payload = outcome.inspection;
        if let Some(object) = payload.as_object_mut() {
            object.insert("archive_action".to_owned(), outcome.action);
        }
        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(request.target.session_ids.len());
    for target_session_id in &request.target.session_ids {
        results.push(execute_session_archive_batch_result(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            request.dry_run,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: session_batch_payload(
            "session_archive",
            current_session_id,
            request.dry_run,
            request.target.session_ids.len(),
            results,
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn build_session_archive_plan(
    snapshot: &SessionInspectionSnapshot,
) -> Result<SessionArchivePlan, String> {
    if snapshot.session.archived_at.is_some() {
        return Err(format!(
            "session_archive_not_archivable: session `{}` is already archived",
            snapshot.session.session_id
        ));
    }
    if !session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_archive_not_archivable: session `{}` is not terminal",
            snapshot.session.session_id
        ));
    }

    Ok(SessionArchivePlan {
        expected_state: snapshot.session.state,
    })
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_archive_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    archive_plan: &SessionArchivePlan,
) -> Result<SessionToolActionOutcome, String> {
    let transitioned = repo.transition_session_with_event_if_current(
        target_session_id,
        crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
            expected_state: archive_plan.expected_state,
            next_state: archive_plan.expected_state,
            last_error: snapshot.session.last_error.clone(),
            event_kind: "session_archived".to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            event_payload_json: json!({
                "previous_state": archive_plan.expected_state.as_str(),
                "hides_from_sessions_list": true,
            }),
        },
    )?;
    if transitioned.is_none() {
        let latest = repo
            .load_session_summary_with_legacy_fallback(target_session_id)?
            .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        return Err(format!(
            "session_archive_state_changed: session `{target_session_id}` is no longer archivable from state `{}`",
            latest.state.as_str()
        ));
    }

    let archived_snapshot = inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    Ok(SessionToolActionOutcome {
        inspection: session_inspection_payload(archived_snapshot),
        action: session_archive_action_json(archive_plan),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_archive_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        target_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let inspection = session_inspection_payload(snapshot.clone());
    let archive_plan = match build_session_archive_plan(&snapshot) {
        Ok(plan) => plan,
        Err(error)
            if error.starts_with("session_archive_not_archivable:")
                && error.contains("already archived") =>
        {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_already_archived",
                Some(error),
                None,
                Some(inspection),
            ));
        }
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_archivable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_archive_action_json(&archive_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_archive_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        &archive_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_archive_state_changed:") => {
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspect_visible_session_with_policies(
                    target_session_id,
                    current_session_id,
                    config,
                    tool_config,
                    10,
                )
                .ok()
                .map(session_inspection_payload),
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_archive_action_json(plan: &SessionArchivePlan) -> Value {
    json!({
        "kind": "session_archived",
        "previous_state": plan.expected_state.as_str(),
        "next_state": plan.expected_state.as_str(),
        "hides_from_sessions_list": true,
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
#[allow(dead_code)]
async fn wait_for_single_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let started_at = Instant::now();
    let mut next_after_id = after_id.unwrap_or(0).max(0);
    let mut observed_events = Vec::new();

    loop {
        let observation = observe_visible_session_with_policies(
            target_session_id,
            current_session_id,
            config,
            tool_config,
            event_limit,
            after_id.map(|_| next_after_id),
            event_limit,
        )?;
        let snapshot = observation.inspection;
        if let Some(last_tail_event_id) = observation.tail_events.last().map(|event| event.id) {
            next_after_id = last_tail_event_id;
        }
        observed_events.extend(observation.tail_events);
        if session_state_is_terminal(snapshot.session.state) {
            return Ok(wait_outcome(
                "ok",
                snapshot,
                after_id,
                timeout_ms,
                if after_id.is_some() {
                    observed_events
                } else {
                    Vec::new()
                },
                next_after_id,
            ));
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            return Ok(ToolCoreOutcome {
                status: "timeout".to_owned(),
                payload: wait_payload(
                    snapshot,
                    "timeout",
                    after_id,
                    timeout_ms,
                    if after_id.is_some() {
                        observed_events
                    } else {
                        Vec::new()
                    },
                    next_after_id,
                ),
            });
        }

        let remaining_ms = timeout_ms - elapsed_ms;
        sleep(Duration::from_millis(remaining_ms.min(25))).await;
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
async fn wait_for_session_batch_with_policies(
    target_session_ids: Vec<String>,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let mut results = vec![None; target_session_ids.len()];
    let mut pending = Vec::new();
    for (index, target_session_id) in target_session_ids.into_iter().enumerate() {
        if let Err(error) = ensure_visible(
            &repo,
            current_session_id,
            &target_session_id,
            tool_config.sessions.visibility,
        ) {
            set_session_batch_result(
                &mut results,
                index,
                session_batch_result(
                    target_session_id,
                    "skipped_not_visible",
                    Some(error),
                    None,
                    None,
                ),
            )?;
            continue;
        }
        pending.push(SessionWaitTargetState {
            index,
            session_id: target_session_id,
            next_after_id: after_id.unwrap_or(0).max(0),
            observed_events: Vec::new(),
            latest_inspection: None,
        });
    }
    drop(repo);

    let started_at = Instant::now();
    loop {
        let mut next_pending = Vec::with_capacity(pending.len());
        for mut target in pending.into_iter() {
            let observation = match observe_visible_session_with_policies(
                &target.session_id,
                current_session_id,
                config,
                tool_config,
                event_limit,
                after_id.map(|_| target.next_after_id),
                event_limit,
            ) {
                Ok(observation) => observation,
                Err(error) if is_session_visibility_skip_error(&error) => {
                    set_session_batch_result(
                        &mut results,
                        target.index,
                        session_batch_result(
                            target.session_id,
                            "skipped_not_visible",
                            Some(error),
                            None,
                            None,
                        ),
                    )?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let snapshot = observation.inspection;
            if let Some(last_tail_event_id) = observation.tail_events.last().map(|event| event.id) {
                target.next_after_id = last_tail_event_id;
            }
            target.observed_events.extend(observation.tail_events);
            target.latest_inspection = Some(snapshot.clone());
            if session_state_is_terminal(snapshot.session.state) {
                set_session_batch_result(
                    &mut results,
                    target.index,
                    session_batch_result(
                        target.session_id,
                        "ok",
                        None,
                        None,
                        Some(wait_payload(
                            snapshot,
                            "completed",
                            after_id,
                            timeout_ms,
                            if after_id.is_some() {
                                std::mem::take(&mut target.observed_events)
                            } else {
                                Vec::new()
                            },
                            target.next_after_id,
                        )),
                    ),
                )?;
                continue;
            }
            next_pending.push(target);
        }
        pending = next_pending;

        if pending.is_empty() {
            let results = collect_session_batch_results(results)?;
            return Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: session_wait_batch_payload(
                    current_session_id,
                    after_id,
                    timeout_ms,
                    results,
                ),
            });
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms >= timeout_ms {
            for mut target in pending.into_iter() {
                let snapshot = target.latest_inspection.take().ok_or_else(|| {
                    format!(
                        "session_wait_internal_error: missing pending inspection for `{}`",
                        target.session_id
                    )
                })?;
                set_session_batch_result(
                    &mut results,
                    target.index,
                    session_batch_result(
                        target.session_id,
                        "timeout",
                        None,
                        None,
                        Some(wait_payload(
                            snapshot,
                            "timeout",
                            after_id,
                            timeout_ms,
                            if after_id.is_some() {
                                target.observed_events
                            } else {
                                Vec::new()
                            },
                            target.next_after_id,
                        )),
                    ),
                )?;
            }

            let results = collect_session_batch_results(results)?;
            return Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: session_wait_batch_payload(
                    current_session_id,
                    after_id,
                    timeout_ms,
                    results,
                ),
            });
        }

        let remaining_ms = timeout_ms - elapsed_ms;
        sleep(Duration::from_millis(remaining_ms.min(25))).await;
    }
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
    let delegate_events = load_delegate_lifecycle_events(&repo, &session)?;

    Ok(SessionObservationSnapshot {
        inspection: SessionInspectionSnapshot {
            session,
            terminal_outcome,
            recent_events,
            delegate_events,
        },
        tail_events,
    })
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_lifecycle_events(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
) -> Result<Vec<SessionEventRecord>, String> {
    if session.kind != SessionKind::DelegateChild {
        return Ok(Vec::new());
    }
    repo.list_delegate_lifecycle_events(&session.session_id)
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
        snapshot.delegate_events.as_slice(),
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
            "archived": snapshot.session.archived_at.is_some(),
            "archived_at": snapshot.session.archived_at,
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
fn execute_session_status_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        target_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        5,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };

    Ok(session_batch_result(
        target_session_id.to_owned(),
        "ok",
        None,
        None,
        Some(session_inspection_payload(snapshot)),
    ))
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_recover_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        target_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let inspection = session_inspection_payload(snapshot.clone());
    let recover_plan = match build_session_recover_plan(&snapshot, current_unix_ts()) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_recoverable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_recovery_action_json(&recover_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_recover_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        &recover_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_recover_state_changed:") => {
            let inspection = match inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            ) {
                Ok(snapshot) => Some(session_inspection_payload(snapshot)),
                Err(_) => None,
            };
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspection,
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_recover_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    recover_plan: &SessionRecoverPlan,
) -> Result<SessionToolActionOutcome, String> {
    let recovery_error = session_recovery_error(recover_plan);
    let outcome = delegate_error_outcome(
        snapshot.session.session_id.clone(),
        snapshot.session.label.clone(),
        recovery_error.clone(),
        recover_plan.elapsed_seconds.saturating_mul(1_000),
    );
    let outcome_status = outcome.status.clone();
    let outcome_payload = outcome.payload;
    let event_payload_json = match recover_plan.recovery_kind {
        RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED => {
            let Some(queued_at) = recover_plan.queued_at else {
                return Err(format!(
                    "session_recover_not_recoverable: session `{target_session_id}` is missing queued timestamp"
                ));
            };
            build_queued_async_overdue_recovery_payload(
                snapshot.session.label.as_deref(),
                queued_at,
                recover_plan.elapsed_seconds,
                recover_plan.timeout_seconds,
                recover_plan.deadline_at,
                &recovery_error,
            )
        }
        RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED => {
            build_running_async_overdue_recovery_payload(
                snapshot.session.label.as_deref(),
                recover_plan.queued_at,
                recover_plan.started_at,
                recover_plan.reference,
                recover_plan.elapsed_seconds,
                recover_plan.timeout_seconds,
                recover_plan.deadline_at,
                &recovery_error,
            )
        }
        other => {
            return Err(format!(
                "session_recover_not_supported: unsupported recovery kind `{other}`"
            ));
        }
    };
    let finalized = repo.finalize_session_terminal_if_current(
        target_session_id,
        recover_plan.expected_state,
        crate::session::repository::FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some(recovery_error),
            event_kind: RECOVERY_EVENT_KIND.to_owned(),
            actor_session_id: Some(current_session_id.to_owned()),
            event_payload_json,
            outcome_status,
            outcome_payload_json: outcome_payload,
        },
    )?;
    if finalized.is_none() {
        let latest = repo
            .load_session_summary_with_legacy_fallback(target_session_id)?
            .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
        return Err(format!(
            "session_recover_state_changed: session `{target_session_id}` is no longer recoverable from state `{}`",
            latest.state.as_str()
        ));
    }
    let recovered_snapshot = inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    )?;
    Ok(SessionToolActionOutcome {
        inspection: session_inspection_payload(recovered_snapshot),
        action: session_recovery_action_json(recover_plan),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_recovery_action_json(plan: &SessionRecoverPlan) -> Value {
    json!({
        "kind": plan.recovery_kind,
        "previous_state": plan.expected_state.as_str(),
        "next_state": "failed",
        "reference": plan.reference,
        "elapsed_seconds": plan.elapsed_seconds,
        "timeout_seconds": plan.timeout_seconds,
        "deadline_at": plan.deadline_at,
    })
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
    let lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.delegate_events.as_slice(),
        now_ts,
    )
    .ok_or_else(|| {
        format!(
            "session_recover_not_recoverable: session `{}` is missing delegate lifecycle metadata",
            snapshot.session.session_id
        )
    })?;
    if lifecycle.mode != "async" {
        return Err(format!(
            "session_recover_not_recoverable: session `{}` is not an overdue async child",
            snapshot.session.session_id
        ));
    }
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

    match (snapshot.session.state, lifecycle.phase) {
        (SessionState::Ready, "queued") => {
            let queued_at = lifecycle.queued_at.ok_or_else(|| {
                format!(
                    "session_recover_not_recoverable: session `{}` is missing queued timestamp",
                    snapshot.session.session_id
                )
            })?;
            Ok(SessionRecoverPlan {
                expected_state: SessionState::Ready,
                recovery_kind: RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
                reference: "queued",
                queued_at: Some(queued_at),
                started_at: lifecycle.started_at,
                elapsed_seconds: staleness.elapsed_seconds,
                timeout_seconds,
                deadline_at: staleness.deadline_at,
            })
        }
        (SessionState::Running, "running") => Ok(SessionRecoverPlan {
            expected_state: SessionState::Running,
            recovery_kind: RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED,
            reference: staleness.reference,
            queued_at: lifecycle.queued_at,
            started_at: lifecycle.started_at,
            elapsed_seconds: staleness.elapsed_seconds,
            timeout_seconds,
            deadline_at: staleness.deadline_at,
        }),
        _ => Err(format!(
            "session_recover_not_recoverable: session `{}` is not an overdue async child",
            snapshot.session.session_id
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_recovery_error(plan: &SessionRecoverPlan) -> String {
    match plan.recovery_kind {
        RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED => format!(
            "delegate_async_queued_overdue_marked_failed: queued delegate child exceeded timeout after {}s (threshold {}s)",
            plan.elapsed_seconds, plan.timeout_seconds
        ),
        RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED => format!(
            "delegate_async_running_overdue_marked_failed: running delegate child exceeded timeout after {}s (threshold {}s)",
            plan.elapsed_seconds, plan.timeout_seconds
        ),
        other => {
            format!("session_recover_unsupported_kind: unsupported session recovery kind `{other}`")
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_cancel_batch_result(
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    dry_run: bool,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        target_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        target_session_id,
        current_session_id,
        config,
        tool_config,
        10,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let inspection = session_inspection_payload(snapshot.clone());
    let cancel_plan = match build_session_cancel_plan(&snapshot) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_not_cancellable",
                Some(error),
                None,
                Some(inspection),
            ));
        }
    };
    let action = session_cancel_action_json(&cancel_plan);
    if dry_run {
        return Ok(session_batch_result(
            target_session_id.to_owned(),
            "would_apply",
            None,
            Some(action),
            Some(inspection),
        ));
    }

    match apply_session_cancel_plan(
        &repo,
        target_session_id,
        current_session_id,
        config,
        tool_config,
        &snapshot,
        cancel_plan,
    ) {
        Ok(outcome) => Ok(session_batch_result(
            target_session_id.to_owned(),
            "applied",
            None,
            Some(outcome.action),
            Some(outcome.inspection),
        )),
        Err(error) if error.starts_with("session_cancel_state_changed:") => {
            let inspection = match inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            ) {
                Ok(snapshot) => Some(session_inspection_payload(snapshot)),
                Err(_) => None,
            };
            Ok(session_batch_result(
                target_session_id.to_owned(),
                "skipped_state_changed",
                Some(error),
                Some(action),
                inspection,
            ))
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_cancel_plan(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
    config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
    snapshot: &SessionInspectionSnapshot,
    cancel_plan: SessionCancelPlan,
) -> Result<SessionToolActionOutcome, String> {
    match cancel_plan {
        SessionCancelPlan::Queued => {
            let cancel_error = delegate_cancelled_error(DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED);
            let outcome = delegate_error_outcome(
                snapshot.session.session_id.clone(),
                snapshot.session.label.clone(),
                cancel_error.clone(),
                0,
            );
            let outcome_status = outcome.status.clone();
            let outcome_payload = outcome.payload;
            let finalized = repo.finalize_session_terminal_if_current(
                target_session_id,
                SessionState::Ready,
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: SessionState::Failed,
                    last_error: Some(cancel_error),
                    event_kind: DELEGATE_CANCELLED_EVENT_KIND.to_owned(),
                    actor_session_id: Some(current_session_id.to_owned()),
                    event_payload_json: json!({
                        "reference": "queued",
                        "cancel_reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
                    }),
                    outcome_status,
                    outcome_payload_json: outcome_payload,
                },
            )?;
            if finalized.is_none() {
                let latest = repo
                    .load_session_summary_with_legacy_fallback(target_session_id)?
                    .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
                return Err(format!(
                    "session_cancel_state_changed: session `{target_session_id}` is no longer cancellable from state `{}`",
                    latest.state.as_str()
                ));
            }

            let cancelled_snapshot = inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            )?;
            Ok(SessionToolActionOutcome {
                inspection: session_inspection_payload(cancelled_snapshot),
                action: session_cancel_action_json(&SessionCancelPlan::Queued),
            })
        }
        SessionCancelPlan::Running => {
            let requested = repo.transition_session_with_event_if_current(
                target_session_id,
                crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Running,
                    next_state: SessionState::Running,
                    last_error: None,
                    event_kind: DELEGATE_CANCEL_REQUESTED_EVENT_KIND.to_owned(),
                    actor_session_id: Some(current_session_id.to_owned()),
                    event_payload_json: json!({
                        "reference": "running",
                        "cancel_reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
                    }),
                },
            )?;
            if requested.is_none() {
                let latest = repo
                    .load_session_summary_with_legacy_fallback(target_session_id)?
                    .ok_or_else(|| format!("session_not_found: `{target_session_id}`"))?;
                return Err(format!(
                    "session_cancel_state_changed: session `{target_session_id}` is no longer cancellable from state `{}`",
                    latest.state.as_str()
                ));
            }

            let requested_snapshot = inspect_visible_session_with_policies(
                target_session_id,
                current_session_id,
                config,
                tool_config,
                10,
            )?;
            Ok(SessionToolActionOutcome {
                inspection: session_inspection_payload(requested_snapshot),
                action: session_cancel_action_json(&SessionCancelPlan::Running),
            })
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_cancel_action_json(plan: &SessionCancelPlan) -> Value {
    match plan {
        SessionCancelPlan::Queued => json!({
            "kind": "queued_async_cancelled",
            "previous_state": "ready",
            "next_state": "failed",
            "reference": "queued",
            "reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
        }),
        SessionCancelPlan::Running => json!({
            "kind": "running_async_cancel_requested",
            "previous_state": "running",
            "next_state": "running",
            "reference": "running",
            "reason": DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED,
        }),
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_session_cancel_plan(
    snapshot: &SessionInspectionSnapshot,
) -> Result<SessionCancelPlan, String> {
    if snapshot.session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session_cancel_not_supported: session `{}` is not a delegate child",
            snapshot.session.session_id
        ));
    }
    if snapshot.terminal_outcome.is_some() || session_state_is_terminal(snapshot.session.state) {
        return Err(format!(
            "session_cancel_not_cancellable: session `{}` is already terminal",
            snapshot.session.session_id
        ));
    }
    let lifecycle = session_delegate_lifecycle_at(
        &snapshot.session,
        snapshot.delegate_events.as_slice(),
        current_unix_ts(),
    )
    .ok_or_else(|| {
        format!(
            "session_cancel_not_cancellable: session `{}` is missing delegate lifecycle metadata",
            snapshot.session.session_id
        )
    })?;
    if lifecycle.mode != "async" {
        return Err(format!(
            "session_cancel_not_supported: session `{}` is not an async delegate child",
            snapshot.session.session_id
        ));
    }
    match (snapshot.session.state, lifecycle.phase) {
        (SessionState::Ready, "queued") => Ok(SessionCancelPlan::Queued),
        (SessionState::Running, "running") => Ok(SessionCancelPlan::Running),
        _ => Err(format!(
            "session_cancel_not_cancellable: session `{}` is not queued or running",
            snapshot.session.session_id
        )),
    }
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
    let mut execution = None;
    let mut cancellation = None;
    for event in recent_events {
        match event.event_kind.as_str() {
            "delegate_queued" => {
                queued_at = Some(event.ts);
                execution = execution.or_else(|| {
                    ConstrainedSubagentExecution::from_event_payload(&event.payload_json)
                });
                queued_timeout_seconds = event
                    .payload_json
                    .get("timeout_seconds")
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        execution
                            .as_ref()
                            .map(|execution| execution.timeout_seconds)
                    });
            }
            "delegate_started" => {
                started_at = Some(event.ts);
                execution = execution.or_else(|| {
                    ConstrainedSubagentExecution::from_event_payload(&event.payload_json)
                });
                started_timeout_seconds = event
                    .payload_json
                    .get("timeout_seconds")
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        execution
                            .as_ref()
                            .map(|execution| execution.timeout_seconds)
                    });
            }
            DELEGATE_CANCEL_REQUESTED_EVENT_KIND => {
                let reason = event
                    .payload_json
                    .get("cancel_reason")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED)
                    .to_owned();
                let reference = event
                    .payload_json
                    .get("reference")
                    .and_then(Value::as_str)
                    .filter(|value| *value == "running")
                    .unwrap_or("running");
                cancellation = Some(SessionDelegateCancellationRecord {
                    state: "requested",
                    reference: reference.to_owned(),
                    requested_at: event.ts,
                    reason,
                });
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
    let mode = execution
        .as_ref()
        .map(|execution| match execution.mode {
            crate::conversation::ConstrainedSubagentMode::Async => "async",
            crate::conversation::ConstrainedSubagentMode::Inline => "inline",
        })
        .unwrap_or_else(|| {
            if queued_at.is_some() || matches!(session.state, SessionState::Ready) {
                "async"
            } else {
                "inline"
            }
        });
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
        execution,
        staleness,
        cancellation: if session.state == SessionState::Running {
            cancellation
        } else {
            None
        },
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
        "execution": lifecycle.execution,
        "staleness": lifecycle.staleness.map(session_delegate_staleness_json),
        "cancellation": lifecycle
            .cancellation
            .map(session_delegate_cancellation_json),
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
fn session_delegate_cancellation_json(cancellation: SessionDelegateCancellationRecord) -> Value {
    json!({
        "state": cancellation.state,
        "reference": cancellation.reference,
        "requested_at": cancellation.requested_at,
        "reason": cancellation.reason,
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

#[cfg(feature = "memory-sqlite")]
fn resolve_session_tool_policy_target_session_id(
    payload: &Value,
    current_session_id: &str,
) -> Result<String, String> {
    Ok(optional_payload_string(payload, "session_id")
        .unwrap_or_else(|| current_session_id.to_owned()))
}

#[cfg(feature = "memory-sqlite")]
fn ensure_policy_target_session_exists(
    repo: &SessionRepository,
    target_session_id: &str,
    current_session_id: &str,
) -> Result<(), String> {
    let existing_summary = repo.load_session_summary_with_legacy_fallback(target_session_id)?;
    if existing_summary.is_some() {
        return Ok(());
    }
    if target_session_id != current_session_id {
        return Err(format!("session_not_found: `{target_session_id}`"));
    }

    repo.ensure_session(NewSessionRecord {
        session_id: target_session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: SessionState::Ready,
    })?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn session_tool_policy_root_tool_view(
    tool_config: &ToolConfig,
    runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> ToolView {
    crate::tools::runtime_tool_view_with_runtime_config(tool_config, runtime_config)
}

#[cfg(feature = "memory-sqlite")]
fn session_tool_policy_base_tool_view(
    repo: &SessionRepository,
    session_id: &str,
    tool_config: &ToolConfig,
) -> Result<ToolView, String> {
    if let Some(session) = repo.load_session(session_id)? {
        if session.parent_session_id.is_some() {
            let depth = match repo.session_lineage_depth(session_id) {
                Ok(depth) => depth,
                Err(error)
                    if error.starts_with("session_lineage_broken:")
                        || error.starts_with("session_lineage_cycle_detected:") =>
                {
                    return Ok(super::delegate_child_tool_view_for_config_with_delegate(
                        tool_config,
                        false,
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "compute session lineage depth for session tool policy failed: {error}"
                    ));
                }
            };
            let allow_nested_delegate = depth < tool_config.delegate.max_depth;
            return Ok(super::delegate_child_tool_view_for_config_with_delegate(
                tool_config,
                allow_nested_delegate,
            ));
        }
    } else if repo
        .load_session_summary_with_legacy_fallback(session_id)?
        .is_some_and(|session| session.kind == SessionKind::DelegateChild)
    {
        return Ok(super::delegate_child_tool_view_for_config(tool_config));
    }

    let runtime_config = crate::tools::runtime_config::get_tool_runtime_config();
    let root_tool_view = session_tool_policy_root_tool_view(tool_config, runtime_config);
    Ok(root_tool_view)
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_tool_policy_to_tool_view(
    base_tool_view: &ToolView,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> ToolView {
    let Some(session_tool_policy) = session_tool_policy else {
        return base_tool_view.clone();
    };
    if session_tool_policy.requested_tool_ids.is_empty() {
        return base_tool_view.clone();
    }

    let requested_tool_view =
        ToolView::from_tool_names(session_tool_policy.requested_tool_ids.iter());
    base_tool_view.intersect(&requested_tool_view)
}

#[cfg(feature = "memory-sqlite")]
fn load_session_delegate_runtime_narrowing(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<ToolRuntimeNarrowing>, String> {
    let events = repo.list_delegate_lifecycle_events(session_id)?;
    let execution = events.into_iter().rev().find_map(|event| {
        matches!(
            event.event_kind.as_str(),
            "delegate_queued" | "delegate_started"
        )
        .then(|| ConstrainedSubagentExecution::from_event_payload(&event.payload_json))
        .flatten()
    });
    Ok(execution.and_then(|execution| {
        (!execution.runtime_narrowing.is_empty()).then_some(execution.runtime_narrowing)
    }))
}

#[cfg(feature = "memory-sqlite")]
fn merge_session_tool_policy_runtime_narrowing(
    delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> Option<ToolRuntimeNarrowing> {
    let policy_runtime_narrowing = session_tool_policy.and_then(|policy| {
        (!policy.runtime_narrowing.is_empty()).then_some(policy.runtime_narrowing.clone())
    });

    match (delegate_runtime_narrowing, policy_runtime_narrowing) {
        (Some(delegate_runtime_narrowing), Some(policy_runtime_narrowing)) => {
            Some(delegate_runtime_narrowing.intersect(&policy_runtime_narrowing))
        }
        (Some(delegate_runtime_narrowing), None) => Some(delegate_runtime_narrowing),
        (None, Some(policy_runtime_narrowing)) => Some(policy_runtime_narrowing),
        (None, None) => None,
    }
}

#[cfg(feature = "memory-sqlite")]
fn tool_view_names(tool_view: &ToolView) -> Vec<String> {
    tool_view.tool_names().map(str::to_owned).collect()
}

#[cfg(feature = "memory-sqlite")]
fn runtime_narrowing_json(runtime_narrowing: Option<ToolRuntimeNarrowing>) -> Value {
    match runtime_narrowing {
        Some(runtime_narrowing) => serde_json::to_value(runtime_narrowing).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_session_tool_policy_status_payload(
    repo: &SessionRepository,
    target_session_id: &str,
    tool_config: &ToolConfig,
) -> Result<Value, String> {
    let session_tool_policy = repo.load_session_tool_policy(target_session_id)?;
    let base_tool_view = session_tool_policy_base_tool_view(repo, target_session_id, tool_config)?;
    let effective_tool_view =
        apply_session_tool_policy_to_tool_view(&base_tool_view, session_tool_policy.as_ref());
    let delegate_runtime_narrowing =
        load_session_delegate_runtime_narrowing(repo, target_session_id)?;
    let effective_runtime_narrowing = merge_session_tool_policy_runtime_narrowing(
        delegate_runtime_narrowing.clone(),
        session_tool_policy.as_ref(),
    );
    let requested_tool_ids = session_tool_policy
        .as_ref()
        .map(|policy| policy.requested_tool_ids.clone())
        .unwrap_or_default();
    let requested_runtime_narrowing = session_tool_policy.as_ref().and_then(|policy| {
        (!policy.runtime_narrowing.is_empty()).then_some(policy.runtime_narrowing.clone())
    });
    let updated_at = session_tool_policy.as_ref().map(|policy| policy.updated_at);

    Ok(json!({
        "has_policy": session_tool_policy.is_some(),
        "updated_at": updated_at,
        "requested_tool_ids": requested_tool_ids,
        "base_tool_ids": tool_view_names(&base_tool_view),
        "effective_tool_ids": tool_view_names(&effective_tool_view),
        "requested_runtime_narrowing": runtime_narrowing_json(requested_runtime_narrowing),
        "delegate_runtime_narrowing": runtime_narrowing_json(delegate_runtime_narrowing),
        "effective_runtime_narrowing": runtime_narrowing_json(effective_runtime_narrowing),
    }))
}

#[cfg(feature = "memory-sqlite")]
fn resolve_session_tool_policy_tool_ids(
    repo: &SessionRepository,
    session_id: &str,
    tool_config: &ToolConfig,
    raw_tool_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let base_tool_view = session_tool_policy_base_tool_view(repo, session_id, tool_config)?;
    let mut normalized_tool_ids = BTreeMap::new();

    for raw_tool_id in raw_tool_ids {
        let canonical_tool_id = crate::tools::canonical_tool_name(&raw_tool_id).to_owned();
        if !base_tool_view.contains(&canonical_tool_id) {
            return Err(format!(
                "session_tool_policy_set_invalid_tool_id: `{raw_tool_id}` is not available in session `{session_id}`"
            ));
        }
        normalized_tool_ids.insert(canonical_tool_id.clone(), canonical_tool_id);
    }

    Ok(normalized_tool_ids.into_values().collect())
}

#[cfg(feature = "memory-sqlite")]
fn normalize_session_tool_runtime_narrowing(
    mut runtime_narrowing: ToolRuntimeNarrowing,
) -> ToolRuntimeNarrowing {
    // Persisted session policies are only allowed to tighten fetch access, never widen it.
    if runtime_narrowing.web_fetch.allow_private_hosts == Some(true) {
        runtime_narrowing.web_fetch.allow_private_hosts = None;
    }
    runtime_narrowing.browser.max_sessions = runtime_narrowing
        .browser
        .max_sessions
        .map(|value| value.max(1));
    runtime_narrowing.browser.max_links = runtime_narrowing
        .browser
        .max_links
        .map(|value| value.max(1));
    runtime_narrowing.browser.max_text_chars = runtime_narrowing
        .browser
        .max_text_chars
        .map(|value| value.max(1));
    runtime_narrowing.web_fetch.timeout_seconds = runtime_narrowing
        .web_fetch
        .timeout_seconds
        .map(|value| value.max(1));
    runtime_narrowing.web_fetch.max_bytes = runtime_narrowing
        .web_fetch
        .max_bytes
        .map(|value| value.max(1));
    runtime_narrowing.web_fetch.max_redirects = runtime_narrowing
        .web_fetch
        .max_redirects
        .map(|value| value.max(1));
    if !runtime_narrowing.web_fetch.allowed_domains.is_empty() {
        runtime_narrowing.web_fetch.enforce_allowed_domains = true;
    }
    runtime_narrowing
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_tool_policy_set_request(
    payload: &Value,
    current_session_id: &str,
) -> Result<SessionToolPolicySetRequest, String> {
    let session_id = resolve_session_tool_policy_target_session_id(payload, current_session_id)?;
    let tool_ids = optional_payload_session_tool_policy_tool_ids(payload, "tool_ids")?;
    let runtime_narrowing =
        optional_payload_session_tool_runtime_narrowing(payload, "runtime_narrowing")?;
    if tool_ids.is_none() && runtime_narrowing.is_none() {
        return Err(
            "session_tool_policy_set requires payload.tool_ids or payload.runtime_narrowing"
                .to_owned(),
        );
    }

    Ok(SessionToolPolicySetRequest {
        session_id,
        tool_ids,
        runtime_narrowing,
    })
}

#[cfg(feature = "memory-sqlite")]
fn normalize_required_session_id(session_id: &str) -> Result<String, String> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err("session tool requires payload.session_id".to_owned());
    }
    Ok(trimmed.to_owned())
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_target_request(payload: &Value) -> Result<SessionTargetRequest, String> {
    let single = optional_payload_string(payload, "session_id");
    let batch = optional_payload_string_array(payload, "session_ids")?;

    match (single, batch) {
        (Some(session_id), None) => Ok(SessionTargetRequest {
            session_ids: vec![normalize_required_session_id(&session_id)?],
            legacy_single: true,
        }),
        (None, Some(session_ids)) => Ok(SessionTargetRequest {
            session_ids,
            legacy_single: false,
        }),
        (Some(_), Some(_)) => Err(
            "session tool requires exactly one of payload.session_id or payload.session_ids"
                .to_owned(),
        ),
        (None, None) => {
            Err("session tool requires payload.session_id or payload.session_ids".to_owned())
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_mutation_request(payload: &Value) -> Result<SessionMutationRequest, String> {
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(SessionMutationRequest {
        target: parse_session_target_request(payload)?,
        dry_run,
    })
}

#[cfg(feature = "memory-sqlite")]
fn legacy_single_session_id(session_ids: &[String]) -> Result<&str, String> {
    session_ids.first().map(String::as_str).ok_or_else(|| {
        "session_tool_internal_error: legacy single request missing session id".to_owned()
    })
}

#[cfg(feature = "memory-sqlite")]
fn parse_sessions_list_request(
    payload: &Value,
    tool_config: &ToolConfig,
) -> Result<SessionsListRequest, String> {
    Ok(SessionsListRequest {
        limit: optional_payload_limit(
            payload,
            "limit",
            tool_config.sessions.list_limit,
            tool_config.sessions.list_limit,
        ),
        state: optional_payload_session_state(payload, "state")?,
        kind: optional_payload_session_kind(payload, "kind")?,
        parent_session_id: optional_payload_string(payload, "parent_session_id"),
        overdue_only: payload
            .get("overdue_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        include_archived: payload
            .get("include_archived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        include_delegate_lifecycle: payload
            .get("include_delegate_lifecycle")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_session_state(
    payload: &Value,
    field: &str,
) -> Result<Option<SessionState>, String> {
    let Some(raw) = optional_payload_string(payload, field) else {
        return Ok(None);
    };
    match raw.as_str() {
        "ready" => Ok(Some(SessionState::Ready)),
        "running" => Ok(Some(SessionState::Running)),
        "completed" => Ok(Some(SessionState::Completed)),
        "failed" => Ok(Some(SessionState::Failed)),
        "timed_out" => Ok(Some(SessionState::TimedOut)),
        _ => Err(format!("invalid session tool payload.{field}: `{raw}`")),
    }
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_session_kind(
    payload: &Value,
    field: &str,
) -> Result<Option<SessionKind>, String> {
    let Some(raw) = optional_payload_string(payload, field) else {
        return Ok(None);
    };
    match raw.as_str() {
        "root" => Ok(Some(SessionKind::Root)),
        "delegate_child" => Ok(Some(SessionKind::DelegateChild)),
        _ => Err(format!("invalid session tool payload.{field}: `{raw}`")),
    }
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_string_array(
    payload: &Value,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| {
        format!("session tool requires payload.{field} to be a non-empty array of strings")
    })?;
    if values.is_empty() {
        return Err(format!(
            "session tool requires payload.{field} to be a non-empty array of strings"
        ));
    }

    let mut session_ids = Vec::with_capacity(values.len());
    for value in values {
        let Some(session_id) = value.as_str() else {
            return Err(format!(
                "session tool requires payload.{field} to be a non-empty array of strings"
            ));
        };
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "session tool requires payload.{field} to be a non-empty array of strings"
            ));
        }
        session_ids.push(trimmed.to_owned());
    }
    Ok(Some(session_ids))
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_session_tool_policy_tool_ids(
    payload: &Value,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    let values = value.as_array().ok_or_else(|| {
        format!("session tool requires payload.{field} to be an array of strings")
    })?;

    let mut tool_ids = Vec::with_capacity(values.len());
    for value in values {
        let Some(tool_id) = value.as_str() else {
            return Err(format!(
                "session tool requires payload.{field} to be an array of strings"
            ));
        };
        let trimmed = tool_id.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "session tool requires payload.{field} to be an array of strings"
            ));
        }
        tool_ids.push(trimmed.to_owned());
    }
    Ok(Some(tool_ids))
}

#[cfg(feature = "memory-sqlite")]
fn optional_payload_session_tool_runtime_narrowing(
    payload: &Value,
    field: &str,
) -> Result<Option<ToolRuntimeNarrowing>, String> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    let runtime_narrowing: ToolRuntimeNarrowing = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid session tool payload.{field}: {error}"))?;
    let runtime_narrowing = normalize_session_tool_runtime_narrowing(runtime_narrowing);
    Ok(Some(runtime_narrowing))
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_payload(
    tool: &str,
    current_session_id: &str,
    dry_run: bool,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    session_batch_payload_with_optional_dry_run(
        tool,
        current_session_id,
        requested_count,
        results,
        Some(dry_run),
    )
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_payload_without_dry_run(
    tool: &str,
    current_session_id: &str,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    session_batch_payload_with_optional_dry_run(
        tool,
        current_session_id,
        requested_count,
        results,
        None,
    )
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_payload_with_optional_dry_run(
    tool: &str,
    current_session_id: &str,
    requested_count: usize,
    results: Vec<SessionBatchResultRecord>,
    dry_run: Option<bool>,
) -> Value {
    let mut result_counts = BTreeMap::<&'static str, usize>::new();
    for result in &results {
        *result_counts.entry(result.result).or_default() += 1;
    }

    let mut payload = json!({
        "tool": tool,
        "current_session_id": current_session_id,
        "requested_count": requested_count,
        "result_counts": result_counts,
        "results": results.into_iter().map(session_batch_result_json).collect::<Vec<_>>(),
    });
    if let Some(dry_run) = dry_run
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("dry_run".to_owned(), Value::Bool(dry_run));
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn session_wait_batch_payload(
    current_session_id: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    results: Vec<SessionBatchResultRecord>,
) -> Value {
    let mut payload = session_batch_payload_without_dry_run(
        "session_wait",
        current_session_id,
        results.len(),
        results,
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("timeout_ms".to_owned(), Value::from(timeout_ms));
        object.insert(
            "after_id".to_owned(),
            after_id.map(Value::from).unwrap_or(Value::Null),
        );
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_result(
    session_id: String,
    result: &'static str,
    message: Option<String>,
    action: Option<Value>,
    inspection: Option<Value>,
) -> SessionBatchResultRecord {
    SessionBatchResultRecord {
        session_id,
        result,
        message,
        action,
        inspection,
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn wait_outcome(
    status: &str,
    snapshot: SessionInspectionSnapshot,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: status.to_owned(),
        payload: wait_payload(
            snapshot,
            if status == "ok" {
                "completed"
            } else {
                "timeout"
            },
            after_id,
            timeout_ms,
            observed_events,
            next_after_id,
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
fn wait_payload(
    snapshot: SessionInspectionSnapshot,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> Value {
    let next_after_id = match after_id {
        Some(_) => next_after_id,
        None => snapshot
            .recent_events
            .last()
            .map(|event| event.id)
            .unwrap_or(0),
    };
    let events = match after_id {
        Some(_) => observed_events,
        None => snapshot.recent_events.clone(),
    };
    let mut payload = session_inspection_payload(snapshot);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "wait_status".to_owned(),
            Value::String(wait_status.to_owned()),
        );
        object.insert("timeout_ms".to_owned(), Value::from(timeout_ms));
        object.insert(
            "after_id".to_owned(),
            after_id.map(Value::from).unwrap_or(Value::Null),
        );
        object.insert("next_after_id".to_owned(), Value::from(next_after_id));
        object.insert(
            "events".to_owned(),
            Value::Array(
                events
                    .into_iter()
                    .map(session_event_json)
                    .collect::<Vec<_>>(),
            ),
        );
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
fn session_batch_result_json(result: SessionBatchResultRecord) -> Value {
    json!({
        "session_id": result.session_id,
        "result": result.result,
        "message": result.message,
        "action": result.action,
        "inspection": result.inspection,
    })
}

#[cfg(feature = "memory-sqlite")]
fn is_session_visibility_skip_error(error: &str) -> bool {
    error.starts_with("visibility_denied:") || error.starts_with("session_not_found:")
}

#[cfg(feature = "memory-sqlite")]
fn sessions_list_filters_json(request: &SessionsListRequest) -> Value {
    json!({
        "limit": request.limit,
        "state": request.state.map(SessionState::as_str),
        "kind": request.kind.map(SessionKind::as_str),
        "parent_session_id": request.parent_session_id.clone(),
        "overdue_only": request.overdue_only,
        "include_archived": request.include_archived,
        "include_delegate_lifecycle": request.effective_include_delegate_lifecycle(),
    })
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
        "archived": session.archived_at.is_some(),
        "archived_at": session.archived_at,
        "turn_count": session.turn_count,
        "last_turn_at": session.last_turn_at,
        "last_error": session.last_error,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json_with_delegate_lifecycle(
    session: SessionSummaryRecord,
    delegate_lifecycle: Option<SessionDelegateLifecycleRecord>,
    include_delegate_lifecycle: bool,
) -> Value {
    let mut payload = session_summary_json(session);
    if include_delegate_lifecycle && let Some(object) = payload.as_object_mut() {
        object.insert(
            "delegate_lifecycle".to_owned(),
            delegate_lifecycle
                .map(session_delegate_lifecycle_json)
                .unwrap_or(Value::Null),
        );
    }
    payload
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

    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
    use rusqlite::params;
    use serde_json::{Value, json};

    use crate::config::{SessionVisibility, ToolConfig};
    use crate::memory::append_turn_direct;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{
        FinalizeSessionTerminalRequest, NewSessionEvent, NewSessionRecord, SessionEventRecord,
        SessionKind, SessionRepository, SessionState, SessionSummaryRecord,
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
            ..MemoryRuntimeConfig::default()
        }
    }

    fn execute_session_mutation_tool_with_config(
        request: ToolCoreRequest,
        current_session_id: &str,
        config: &MemoryRuntimeConfig,
    ) -> Result<ToolCoreOutcome, String> {
        let mut tool_config = ToolConfig::default();
        tool_config.sessions.allow_mutation = true;
        execute_session_tool_with_policies(request, current_session_id, config, &tool_config)
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

    fn batch_result<'a>(payload: &'a Value, session_id: &str) -> &'a Value {
        payload["results"]
            .as_array()
            .expect("results array")
            .iter()
            .find(|item| item.get("session_id").and_then(Value::as_str) == Some(session_id))
            .unwrap_or_else(|| panic!("missing batch result for session `{session_id}`"))
    }

    #[test]
    fn session_mutation_tools_are_disabled_by_default() {
        let config = isolated_memory_config("session-mutation-disabled");
        for tool_name in ["session_archive", "session_cancel", "session_recover"] {
            let error = execute_session_tool_with_config(
                ToolCoreRequest {
                    tool_name: tool_name.to_owned(),
                    payload: json!({
                        "session_id": "child-session"
                    }),
                },
                "root-session",
                &config,
            )
            .expect_err("session mutation tools should require explicit opt-in");
            let expected_error = format!(
                "app_tool_disabled: session mutation tool `{tool_name}` is disabled by config"
            );
            let matches_expected_error = error.contains(expected_error.as_str());

            assert!(
                matches_expected_error,
                "expected mutation gating error for {tool_name}, got: {error}"
            );
        }
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
    fn sessions_list_filters_visible_sessions_by_state_kind_and_parent() {
        let config = isolated_memory_config("sessions-list-filtered");
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
            session_id: "child-running".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running child");
        repo.create_session(NewSessionRecord {
            session_id: "child-completed".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Completed Child".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create completed child");
        repo.create_session(NewSessionRecord {
            session_id: "grandchild-running".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-running".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Running,
        })
        .expect("create grandchild");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({
                    "state": "running",
                    "kind": "delegate_child",
                    "parent_session_id": "root-session"
                }),
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
        assert_eq!(ids, vec!["child-running"]);
        assert_eq!(outcome.payload["matched_count"], 1);
        assert_eq!(outcome.payload["returned_count"], 1);
    }

    #[test]
    fn sessions_list_excludes_archived_sessions_by_default() {
        let config = isolated_memory_config("sessions-list-excludes-archived");
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
            session_id: "archived-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Archived".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archived child");
        repo.create_session(NewSessionRecord {
            session_id: "visible-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Visible".to_owned()),
            state: SessionState::Running,
        })
        .expect("create visible child");
        for session_id in ["archived-child", "visible-child"] {
            repo.finalize_session_terminal(
                session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({ "result": "ok" }),
                    outcome_status: "ok".to_owned(),
                    outcome_payload_json: json!({ "child_session_id": session_id }),
                },
            )
            .expect("finalize child");
        }

        execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_id": "archived-child"
                }),
            },
            "root-session",
            &config,
        )
        .expect("archive child");

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
        assert!(ids.contains(&"visible-child"));
        assert!(!ids.contains(&"archived-child"));
    }

    #[test]
    fn sessions_list_can_include_archived_sessions_when_requested() {
        let config = isolated_memory_config("sessions-list-include-archived");
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
            session_id: "archived-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Archived".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archived child");
        repo.finalize_session_terminal(
            "archived-child",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({ "result": "ok" }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({ "child_session_id": "archived-child" }),
            },
        )
        .expect("finalize child");
        execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_id": "archived-child"
                }),
            },
            "root-session",
            &config,
        )
        .expect("archive child");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({
                    "include_archived": true
                }),
            },
            "root-session",
            &config,
        )
        .expect("sessions_list outcome");

        let archived = outcome.payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|item| item["session_id"] == "archived-child")
            .expect("archived session");
        assert_eq!(outcome.payload["filters"]["include_archived"], true);
        assert_eq!(archived["archived"], true);
        assert!(archived["archived_at"].is_number());
    }

    #[test]
    fn sessions_list_overdue_only_uses_lifecycle_anchor_events() {
        let config = isolated_memory_config("sessions-list-overdue-only");
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
            session_id: "overdue-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Overdue".to_owned()),
            state: SessionState::Running,
        })
        .expect("create overdue child");
        repo.create_session(NewSessionRecord {
            session_id: "fresh-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Fresh".to_owned()),
            state: SessionState::Running,
        })
        .expect("create fresh child");

        repo.append_event(NewSessionEvent {
            session_id: "overdue-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "timeout_seconds": 30 }),
        })
        .expect("append overdue queued");
        repo.append_event(NewSessionEvent {
            session_id: "overdue-child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "timeout_seconds": 30 }),
        })
        .expect("append overdue started");
        overwrite_session_event_ts(
            &config,
            "overdue-child",
            "delegate_queued",
            super::current_unix_ts() - 120,
        );
        overwrite_session_event_ts(
            &config,
            "overdue-child",
            "delegate_started",
            super::current_unix_ts() - 90,
        );
        for step in 0..20 {
            repo.append_event(NewSessionEvent {
                session_id: "overdue-child".to_owned(),
                event_kind: format!("delegate_progress_{step}"),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({ "step": step }),
            })
            .expect("append overdue progress");
        }

        repo.append_event(NewSessionEvent {
            session_id: "fresh-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "timeout_seconds": 300 }),
        })
        .expect("append fresh queued");
        repo.append_event(NewSessionEvent {
            session_id: "fresh-child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "timeout_seconds": 300 }),
        })
        .expect("append fresh started");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({
                    "kind": "delegate_child",
                    "overdue_only": true
                }),
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
        assert_eq!(ids, vec!["overdue-child"]);
        assert_eq!(outcome.payload["matched_count"], 1);
        assert_eq!(sessions[0]["delegate_lifecycle"]["mode"], "async");
        assert_eq!(sessions[0]["delegate_lifecycle"]["phase"], "running");
        assert_eq!(
            sessions[0]["delegate_lifecycle"]["staleness"]["state"],
            "overdue"
        );
        assert_eq!(
            sessions[0]["delegate_lifecycle"]["staleness"]["reference"],
            "started"
        );
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
    fn session_tool_policy_tools_round_trip_and_clear_policy() {
        let config = isolated_memory_config("session-tool-policy-tools");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");

        let set = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_tool_policy_set".to_owned(),
                payload: json!({
                    "tool_ids": ["tool.search", "session_status"],
                    "runtime_narrowing": {
                        "browser": {
                            "max_sessions": 2,
                        },
                        "web_fetch": {
                            "allowed_domains": ["docs.example.com"],
                            "blocked_domains": ["deny.example.com"],
                            "allow_private_hosts": false,
                        }
                    }
                }),
            },
            "root-session",
            &config,
        )
        .expect("set session tool policy");

        assert_eq!(set.payload["action"], "created");
        assert_eq!(set.payload["policy"]["has_policy"], true);
        assert_eq!(
            set.payload["policy"]["requested_tool_ids"],
            json!(["session_status", "tool.search"])
        );
        assert_eq!(
            set.payload["policy"]["effective_tool_ids"],
            json!(["session_status", "tool.search"])
        );
        assert_eq!(
            set.payload["policy"]["requested_runtime_narrowing"]["browser"]["max_sessions"],
            2
        );
        assert_eq!(
            set.payload["policy"]["effective_runtime_narrowing"]["web_fetch"]["allowed_domains"],
            json!(["docs.example.com"])
        );

        let status = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_tool_policy_status".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
        )
        .expect("session tool policy status");

        assert_eq!(status.payload["policy"]["has_policy"], true);
        assert_eq!(
            status.payload["policy"]["requested_tool_ids"],
            json!(["session_status", "tool.search"])
        );
        assert_eq!(
            status.payload["policy"]["requested_runtime_narrowing"]["web_fetch"]["blocked_domains"],
            json!(["deny.example.com"])
        );

        let clear = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_tool_policy_clear".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
        )
        .expect("clear session tool policy");

        assert_eq!(clear.payload["action"], "cleared");
        assert_eq!(clear.payload["policy"]["has_policy"], false);
        assert_eq!(clear.payload["policy"]["requested_tool_ids"], json!([]));
        assert!(
            clear.payload["policy"]["effective_tool_ids"]
                .as_array()
                .expect("effective tool ids")
                .iter()
                .any(|value| value == "session_status")
        );
    }

    #[test]
    fn session_tool_policy_set_bootstraps_current_root_session_when_missing() {
        let config = isolated_memory_config("session-tool-policy-bootstrap");
        let repo = SessionRepository::new(&config).expect("repository");

        let set = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_tool_policy_set".to_owned(),
                payload: json!({
                    "tool_ids": ["tool.search", "session_status"]
                }),
            },
            "fresh-root-session",
            &config,
        )
        .expect("set session tool policy");

        assert_eq!(set.payload["action"], "created");
        let session = repo
            .load_session("fresh-root-session")
            .expect("load bootstrapped root session")
            .expect("bootstrapped root session");
        assert_eq!(session.kind, SessionKind::Root);
        assert_eq!(session.state, SessionState::Ready);

        let policy = repo
            .load_session_tool_policy("fresh-root-session")
            .expect("load bootstrapped session tool policy")
            .expect("bootstrapped session tool policy");
        assert_eq!(
            policy.requested_tool_ids,
            vec!["session_status".to_owned(), "tool.search".to_owned()]
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn session_tool_policy_root_tool_view_includes_runtime_discovered_feishu_tools() {
        let runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
            feishu: Some(crate::tools::runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    app_id: Some(loongclaw_contracts::SecretRef::Inline(
                        "test-feishu-app-id".to_owned(),
                    )),
                    app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                        "test-feishu-app-secret".to_owned(),
                    )),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig::default(),
            }),
            ..crate::tools::runtime_config::ToolRuntimeConfig::default()
        };
        let tool_config = ToolConfig::default();
        let tool_view = super::session_tool_policy_root_tool_view(&tool_config, &runtime_config);

        assert!(tool_view.contains("feishu.whoami"));
        assert!(tool_view.contains("feishu.messages.send"));
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

        let outcome = execute_session_mutation_tool_with_config(
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

        let error = execute_session_mutation_tool_with_config(
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
    fn session_recover_marks_overdue_running_async_child_failed() {
        let config = isolated_memory_config("session-recover-running-overdue");
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
        overwrite_session_event_ts(
            &config,
            "child-session",
            "delegate_queued",
            super::current_unix_ts() - 120,
        );
        overwrite_session_event_ts(
            &config,
            "child-session",
            "delegate_started",
            super::current_unix_ts() - 90,
        );

        let outcome = execute_session_mutation_tool_with_config(
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
            "running_async_overdue_marked_failed"
        );
        assert_eq!(
            outcome.payload["recovery_action"]["previous_state"],
            "running"
        );
        assert_eq!(outcome.payload["recovery_action"]["reference"], "started");
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
    fn session_recover_rejects_fresh_running_child() {
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

        let error = execute_session_mutation_tool_with_config(
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
    fn session_recover_batch_dry_run_reports_mixed_results_without_mutation() {
        let config = isolated_memory_config("session-recover-batch-dry-run");
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
            session_id: "overdue-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Overdue".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create overdue child");
        repo.create_session(NewSessionRecord {
            session_id: "fresh-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Fresh".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create fresh child");
        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden root");
        repo.append_event(NewSessionEvent {
            session_id: "overdue-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 30
            }),
        })
        .expect("append overdue queued");
        repo.append_event(NewSessionEvent {
            session_id: "fresh-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 60
            }),
        })
        .expect("append fresh queued");
        overwrite_session_event_ts(
            &config,
            "overdue-child",
            "delegate_queued",
            super::current_unix_ts() - 90,
        );

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_ids": ["overdue-child", "fresh-child", "hidden-root"],
                    "dry_run": true
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_recover batch dry_run outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_recover");
        assert_eq!(outcome.payload["dry_run"], true);
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["would_apply"], 1);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_recoverable"],
            1
        );
        assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

        let overdue = batch_result(&outcome.payload, "overdue-child");
        assert_eq!(overdue["result"], "would_apply");
        assert_eq!(
            overdue["action"]["kind"],
            "queued_async_overdue_marked_failed"
        );
        assert_eq!(overdue["inspection"]["session"]["state"], "ready");

        let fresh = batch_result(&outcome.payload, "fresh-child");
        assert_eq!(fresh["result"], "skipped_not_recoverable");
        assert!(
            fresh["message"]
                .as_str()
                .expect("fresh batch message")
                .contains("session_recover_not_recoverable")
        );
        assert_eq!(fresh["inspection"]["session"]["state"], "ready");

        let hidden = batch_result(&outcome.payload, "hidden-root");
        assert_eq!(hidden["result"], "skipped_not_visible");
        assert!(
            hidden["message"]
                .as_str()
                .expect("hidden batch message")
                .contains("visibility_denied")
        );
        assert!(hidden["inspection"].is_null());

        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("overdue-child")
                .expect("load overdue summary")
                .expect("overdue session")
                .state,
            SessionState::Ready
        );
        assert!(
            repo.load_terminal_outcome("overdue-child")
                .expect("load overdue outcome")
                .is_none()
        );
    }

    #[test]
    fn session_recover_batch_apply_reports_partial_success() {
        let config = isolated_memory_config("session-recover-batch-apply");
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
            session_id: "queued-overdue".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Queued Overdue".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create queued overdue");
        repo.create_session(NewSessionRecord {
            session_id: "running-overdue".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running Overdue".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running overdue");
        repo.create_session(NewSessionRecord {
            session_id: "fresh-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Fresh".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create fresh child");
        repo.append_event(NewSessionEvent {
            session_id: "queued-overdue".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "queued work",
                "timeout_seconds": 30
            }),
        })
        .expect("append queued overdue event");
        repo.append_event(NewSessionEvent {
            session_id: "running-overdue".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 30
            }),
        })
        .expect("append running queued event");
        repo.append_event(NewSessionEvent {
            session_id: "running-overdue".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 30
            }),
        })
        .expect("append running started event");
        repo.append_event(NewSessionEvent {
            session_id: "fresh-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "fresh work",
                "timeout_seconds": 60
            }),
        })
        .expect("append fresh event");
        overwrite_session_event_ts(
            &config,
            "queued-overdue",
            "delegate_queued",
            super::current_unix_ts() - 90,
        );
        overwrite_session_event_ts(
            &config,
            "running-overdue",
            "delegate_queued",
            super::current_unix_ts() - 120,
        );
        overwrite_session_event_ts(
            &config,
            "running-overdue",
            "delegate_started",
            super::current_unix_ts() - 90,
        );

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_ids": ["queued-overdue", "running-overdue", "fresh-child"]
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_recover batch apply outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_recover");
        assert_eq!(outcome.payload["dry_run"], false);
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["applied"], 2);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_recoverable"],
            1
        );

        let queued = batch_result(&outcome.payload, "queued-overdue");
        assert_eq!(queued["result"], "applied");
        assert_eq!(queued["inspection"]["session"]["state"], "failed");
        assert_eq!(
            queued["action"]["kind"],
            "queued_async_overdue_marked_failed"
        );
        assert_eq!(
            queued["inspection"]["delegate_lifecycle"]["phase"],
            "failed"
        );

        let running = batch_result(&outcome.payload, "running-overdue");
        assert_eq!(running["result"], "applied");
        assert_eq!(running["inspection"]["session"]["state"], "failed");
        assert_eq!(
            running["action"]["kind"],
            "running_async_overdue_marked_failed"
        );
        assert_eq!(running["action"]["reference"], "started");
        assert_eq!(
            running["inspection"]["recent_events"]
                .as_array()
                .expect("running recent events")
                .last()
                .expect("running latest event")["event_kind"],
            "delegate_recovery_applied"
        );

        let fresh = batch_result(&outcome.payload, "fresh-child");
        assert_eq!(fresh["result"], "skipped_not_recoverable");
        assert_eq!(fresh["inspection"]["session"]["state"], "ready");

        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("queued-overdue")
                .expect("load queued summary")
                .expect("queued session")
                .state,
            SessionState::Failed
        );
        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("running-overdue")
                .expect("load running summary")
                .expect("running session")
                .state,
            SessionState::Failed
        );
        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("fresh-child")
                .expect("load fresh summary")
                .expect("fresh session")
                .state,
            SessionState::Ready
        );
        assert!(
            repo.load_terminal_outcome("queued-overdue")
                .expect("load queued outcome")
                .is_some()
        );
        assert!(
            repo.load_terminal_outcome("running-overdue")
                .expect("load running outcome")
                .is_some()
        );
        assert!(
            repo.load_terminal_outcome("fresh-child")
                .expect("load fresh outcome")
                .is_none()
        );
    }

    #[test]
    fn session_cancel_cancels_queued_async_child() {
        let config = isolated_memory_config("session-cancel-queued");
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

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_cancel outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["session"]["state"], "failed");
        assert_eq!(outcome.payload["terminal_outcome_state"], "present");
        assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
        assert_eq!(
            outcome.payload["cancel_action"]["kind"],
            "queued_async_cancelled"
        );
        assert_eq!(
            outcome.payload["recent_events"]
                .as_array()
                .expect("recent events array")
                .last()
                .expect("latest recent event")["event_kind"],
            "delegate_cancelled"
        );
    }

    #[test]
    fn session_cancel_requests_running_async_child_cancellation() {
        let config = isolated_memory_config("session-cancel-running");
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
                "timeout_seconds": 60
            }),
        })
        .expect("append queued event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 60
            }),
        })
        .expect("append started event");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_cancel outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["session"]["state"], "running");
        assert_eq!(outcome.payload["terminal_outcome_state"], "not_terminal");
        assert_eq!(
            outcome.payload["cancel_action"]["kind"],
            "running_async_cancel_requested"
        );
        assert_eq!(
            outcome.payload["recent_events"]
                .as_array()
                .expect("recent events array")
                .last()
                .expect("latest recent event")["event_kind"],
            "delegate_cancel_requested"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["cancellation"]["state"],
            "requested"
        );
    }

    #[test]
    fn session_cancel_batch_dry_run_reports_mixed_results_without_mutation() {
        let config = isolated_memory_config("session-cancel-batch-dry-run");
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
            session_id: "queued-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Queued".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create queued child");
        repo.create_session(NewSessionRecord {
            session_id: "running-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running child");
        repo.create_session(NewSessionRecord {
            session_id: "completed-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Completed".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create completed child");
        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden root");
        repo.append_event(NewSessionEvent {
            session_id: "queued-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "queued work",
                "timeout_seconds": 60
            }),
        })
        .expect("append queued child event");
        repo.append_event(NewSessionEvent {
            session_id: "running-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 60
            }),
        })
        .expect("append running queued event");
        repo.append_event(NewSessionEvent {
            session_id: "running-child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 60
            }),
        })
        .expect("append running started event");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_ids": ["queued-child", "running-child", "completed-child", "hidden-root"],
                    "dry_run": true
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_cancel batch dry_run outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_cancel");
        assert_eq!(outcome.payload["dry_run"], true);
        assert_eq!(outcome.payload["requested_count"], 4);
        assert_eq!(outcome.payload["result_counts"]["would_apply"], 2);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_cancellable"],
            1
        );
        assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

        let queued = batch_result(&outcome.payload, "queued-child");
        assert_eq!(queued["result"], "would_apply");
        assert_eq!(queued["action"]["kind"], "queued_async_cancelled");
        assert_eq!(queued["inspection"]["session"]["state"], "ready");

        let running = batch_result(&outcome.payload, "running-child");
        assert_eq!(running["result"], "would_apply");
        assert_eq!(running["action"]["kind"], "running_async_cancel_requested");
        assert_eq!(running["inspection"]["session"]["state"], "running");

        let completed = batch_result(&outcome.payload, "completed-child");
        assert_eq!(completed["result"], "skipped_not_cancellable");
        assert_eq!(completed["inspection"]["session"]["state"], "completed");

        let hidden = batch_result(&outcome.payload, "hidden-root");
        assert_eq!(hidden["result"], "skipped_not_visible");
        assert!(hidden["inspection"].is_null());

        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("queued-child")
                .expect("load queued summary")
                .expect("queued session")
                .state,
            SessionState::Ready
        );
        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("running-child")
                .expect("load running summary")
                .expect("running session")
                .state,
            SessionState::Running
        );
        assert!(
            repo.load_terminal_outcome("queued-child")
                .expect("load queued outcome")
                .is_none()
        );
    }

    #[test]
    fn session_cancel_batch_apply_reports_partial_success() {
        let config = isolated_memory_config("session-cancel-batch-apply");
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
            session_id: "queued-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Queued".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create queued child");
        repo.create_session(NewSessionRecord {
            session_id: "running-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running child");
        repo.create_session(NewSessionRecord {
            session_id: "completed-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Completed".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create completed child");
        repo.append_event(NewSessionEvent {
            session_id: "queued-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "queued work",
                "timeout_seconds": 60
            }),
        })
        .expect("append queued child event");
        repo.append_event(NewSessionEvent {
            session_id: "running-child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 60
            }),
        })
        .expect("append running queued event");
        repo.append_event(NewSessionEvent {
            session_id: "running-child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "running work",
                "timeout_seconds": 60
            }),
        })
        .expect("append running started event");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_ids": ["queued-child", "running-child", "completed-child"]
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_cancel batch apply outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_cancel");
        assert_eq!(outcome.payload["dry_run"], false);
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["applied"], 2);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_cancellable"],
            1
        );

        let queued = batch_result(&outcome.payload, "queued-child");
        assert_eq!(queued["result"], "applied");
        assert_eq!(queued["inspection"]["session"]["state"], "failed");
        assert_eq!(queued["action"]["kind"], "queued_async_cancelled");
        assert_eq!(
            queued["inspection"]["recent_events"]
                .as_array()
                .expect("queued recent events")
                .last()
                .expect("queued latest event")["event_kind"],
            "delegate_cancelled"
        );

        let running = batch_result(&outcome.payload, "running-child");
        assert_eq!(running["result"], "applied");
        assert_eq!(running["inspection"]["session"]["state"], "running");
        assert_eq!(running["action"]["kind"], "running_async_cancel_requested");
        assert_eq!(
            running["inspection"]["delegate_lifecycle"]["cancellation"]["state"],
            "requested"
        );

        let completed = batch_result(&outcome.payload, "completed-child");
        assert_eq!(completed["result"], "skipped_not_cancellable");
        assert_eq!(completed["inspection"]["session"]["state"], "completed");

        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("queued-child")
                .expect("load queued summary")
                .expect("queued session")
                .state,
            SessionState::Failed
        );
        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("running-child")
                .expect("load running summary")
                .expect("running session")
                .state,
            SessionState::Running
        );
        assert!(
            repo.load_terminal_outcome("queued-child")
                .expect("load queued outcome")
                .is_some()
        );
        assert!(
            repo.load_terminal_outcome("running-child")
                .expect("load running outcome")
                .is_none()
        );
    }

    #[test]
    fn session_cancel_requested_state_is_visible_in_session_status() {
        let config = isolated_memory_config("session-cancel-status");
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
                "timeout_seconds": 60
            }),
        })
        .expect("append queued event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "timeout_seconds": 60
            }),
        })
        .expect("append started event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_cancel_requested".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "reference": "running",
                "cancel_reason": "operator_requested"
            }),
        })
        .expect("append cancel requested event");

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

        assert_eq!(
            outcome.payload["delegate_lifecycle"]["cancellation"]["state"],
            "requested"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["cancellation"]["reference"],
            "running"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["cancellation"]["reason"],
            "operator_requested"
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
            archived_at: None,
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
                "timeout_seconds": 60,
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 2,
                    "active_children": 0,
                    "max_active_children": 3,
                    "timeout_seconds": 60,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read", "file.write", "file.edit"],
                    "kernel_bound": false,
                    "runtime_narrowing": {
                        "web_fetch": {
                            "allowed_domains": ["docs.example.com"],
                            "allow_private_hosts": false
                        },
                        "browser": {
                            "max_sessions": 1
                        }
                    }
                }
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
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["mode"],
            "async"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["depth"],
            1
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["max_depth"],
            2
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["active_children"],
            0
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["max_active_children"],
            3
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["allow_shell_in_child"],
            false
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["child_tool_allowlist"],
            json!(["file.read", "file.write", "file.edit"])
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["kernel_bound"],
            false
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["web_fetch"]["allowed_domains"],
            json!(["docs.example.com"])
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["web_fetch"]["allow_private_hosts"],
            false
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["execution"]["runtime_narrowing"]["browser"]["max_sessions"],
            1
        );
    }

    #[test]
    fn session_status_uses_delegate_lifecycle_anchor_events_when_recent_window_is_noisy() {
        let config = isolated_memory_config("session-status-lifecycle-noisy-window");
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
            payload_json: json!({ "timeout_seconds": 30 }),
        })
        .expect("append queued event");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({ "timeout_seconds": 30 }),
        })
        .expect("append started event");
        overwrite_session_event_ts(
            &config,
            "child-session",
            "delegate_queued",
            super::current_unix_ts() - 120,
        );
        overwrite_session_event_ts(
            &config,
            "child-session",
            "delegate_started",
            super::current_unix_ts() - 90,
        );
        for step in 0..20 {
            repo.append_event(NewSessionEvent {
                session_id: "child-session".to_owned(),
                event_kind: format!("delegate_progress_{step}"),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({ "step": step }),
            })
            .expect("append progress event");
        }

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
        assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "running");
        assert_eq!(outcome.payload["delegate_lifecycle"]["timeout_seconds"], 30);
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["staleness"]["reference"],
            "started"
        );
        assert_eq!(
            outcome.payload["delegate_lifecycle"]["staleness"]["state"],
            "overdue"
        );
        assert!(outcome.payload["delegate_lifecycle"]["started_at"].is_number());
    }

    #[test]
    fn session_delegate_lifecycle_prefers_execution_mode_when_history_is_partial() {
        let session = SessionSummaryRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Completed,
            created_at: 100,
            updated_at: 120,
            archived_at: None,
            turn_count: 1,
            last_turn_at: Some(120),
            last_error: None,
        };
        let events = vec![
            SessionEventRecord {
                id: 1,
                session_id: "child-session".to_owned(),
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "task": "research",
                    "execution": {
                        "mode": "async",
                        "depth": 1,
                        "max_depth": 2,
                        "active_children": 0,
                        "max_active_children": 3,
                        "timeout_seconds": 60,
                        "allow_shell_in_child": false,
                        "child_tool_allowlist": ["file.read"],
                        "kernel_bound": false
                    }
                }),
                ts: 110,
            },
            SessionEventRecord {
                id: 2,
                session_id: "child-session".to_owned(),
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "terminal_reason": "completed"
                }),
                ts: 120,
            },
        ];

        let lifecycle = super::session_delegate_lifecycle_at(&session, &events, 130)
            .expect("delegate lifecycle");

        assert_eq!(
            lifecycle.mode, "async",
            "persisted execution.mode should win when queued metadata is absent"
        );
        assert_eq!(lifecycle.phase, "completed");
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
        assert!(
            repo.load_session("delegate:legacy-child")
                .expect("load legacy session")
                .is_none()
        );
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
    fn session_status_batch_returns_mixed_visible_and_hidden_results() {
        let config = isolated_memory_config("session-status-batch");
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
            session_id: "grandchild-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("child-session".to_owned()),
            label: Some("Grandchild".to_owned()),
            state: SessionState::Completed,
        })
        .expect("create grandchild");
        repo.create_session(NewSessionRecord {
            session_id: "hidden-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Hidden".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create hidden root");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_ids": ["hidden-root", "grandchild-session", "child-session"]
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status batch outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_status");
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["ok"], 2);
        assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

        let results = outcome.payload["results"]
            .as_array()
            .expect("batch results array");
        let ids: Vec<&str> = results
            .iter()
            .filter_map(|item| item.get("session_id"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            ids,
            vec!["hidden-root", "grandchild-session", "child-session"]
        );

        let hidden = batch_result(&outcome.payload, "hidden-root");
        assert_eq!(hidden["result"], "skipped_not_visible");
        assert!(hidden["inspection"].is_null());
        assert!(
            hidden["message"]
                .as_str()
                .expect("hidden message")
                .contains("visibility_denied")
        );

        let grandchild = batch_result(&outcome.payload, "grandchild-session");
        assert_eq!(grandchild["result"], "ok");
        assert_eq!(grandchild["inspection"]["session"]["state"], "completed");
        assert_eq!(
            grandchild["inspection"]["session"]["session_id"],
            "grandchild-session"
        );

        let child = batch_result(&outcome.payload, "child-session");
        assert_eq!(child["result"], "ok");
        assert_eq!(child["inspection"]["session"]["state"], "running");
        assert_eq!(
            child["inspection"]["terminal_outcome_state"],
            "not_terminal"
        );
    }

    #[test]
    fn session_archive_archives_terminal_visible_session() {
        let config = isolated_memory_config("session-archive-single");
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
        repo.finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "result": "ok"
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "result": "ok"
                }),
            },
        )
        .expect("finalize child");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_archive outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["session"]["session_id"], "child-session");
        assert_eq!(outcome.payload["session"]["state"], "completed");
        assert_eq!(outcome.payload["session"]["archived"], true);
        assert!(outcome.payload["session"]["archived_at"].is_number());
        assert_eq!(
            outcome.payload["archive_action"]["kind"],
            "session_archived"
        );

        let status = execute_session_tool_with_config(
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

        assert_eq!(status.payload["session"]["archived"], true);
        assert!(status.payload["session"]["archived_at"].is_number());
    }

    #[test]
    fn session_archive_batch_dry_run_reports_mixed_results_without_mutation() {
        let config = isolated_memory_config("session-archive-batch-dry-run");
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
            session_id: "ready-to-archive".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Ready".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archivable child");
        repo.create_session(NewSessionRecord {
            session_id: "already-archived".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Archived".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archived child");
        repo.create_session(NewSessionRecord {
            session_id: "running-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running child");

        for session_id in ["ready-to-archive", "already-archived"] {
            repo.finalize_session_terminal(
                session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({ "result": "ok" }),
                    outcome_status: "ok".to_owned(),
                    outcome_payload_json: json!({ "child_session_id": session_id }),
                },
            )
            .expect("finalize child");
        }
        execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_id": "already-archived"
                }),
            },
            "root-session",
            &config,
        )
        .expect("archive already-archived child");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_ids": ["ready-to-archive", "already-archived", "running-child"],
                    "dry_run": true
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_archive batch dry_run outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_archive");
        assert_eq!(outcome.payload["dry_run"], true);
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["would_apply"], 1);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_already_archived"],
            1
        );
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_archivable"],
            1
        );

        let ready = batch_result(&outcome.payload, "ready-to-archive");
        assert_eq!(ready["result"], "would_apply");
        assert_eq!(ready["inspection"]["session"]["archived"], false);
        assert_eq!(ready["action"]["kind"], "session_archived");

        let archived = batch_result(&outcome.payload, "already-archived");
        assert_eq!(archived["result"], "skipped_already_archived");
        assert_eq!(archived["inspection"]["session"]["archived"], true);

        let running = batch_result(&outcome.payload, "running-child");
        assert_eq!(running["result"], "skipped_not_archivable");
        assert_eq!(running["inspection"]["session"]["state"], "running");

        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("ready-to-archive")
                .expect("load ready summary")
                .expect("ready session")
                .archived_at,
            None
        );
    }

    #[test]
    fn session_archive_batch_apply_reports_partial_success() {
        let config = isolated_memory_config("session-archive-batch-apply");
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
            session_id: "ready-to-archive".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Ready".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archivable child");
        repo.create_session(NewSessionRecord {
            session_id: "already-archived".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Archived".to_owned()),
            state: SessionState::Running,
        })
        .expect("create archived child");
        repo.create_session(NewSessionRecord {
            session_id: "running-child".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Running".to_owned()),
            state: SessionState::Running,
        })
        .expect("create running child");

        for session_id in ["ready-to-archive", "already-archived"] {
            repo.finalize_session_terminal(
                session_id,
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some("root-session".to_owned()),
                    event_payload_json: json!({ "result": "ok" }),
                    outcome_status: "ok".to_owned(),
                    outcome_payload_json: json!({ "child_session_id": session_id }),
                },
            )
            .expect("finalize child");
        }
        execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_id": "already-archived"
                }),
            },
            "root-session",
            &config,
        )
        .expect("archive already-archived child");

        let outcome = execute_session_mutation_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_archive".to_owned(),
                payload: json!({
                    "session_ids": ["ready-to-archive", "already-archived", "running-child"]
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_archive batch apply outcome");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "session_archive");
        assert_eq!(outcome.payload["dry_run"], false);
        assert_eq!(outcome.payload["requested_count"], 3);
        assert_eq!(outcome.payload["result_counts"]["applied"], 1);
        assert_eq!(
            outcome.payload["result_counts"]["skipped_already_archived"],
            1
        );
        assert_eq!(
            outcome.payload["result_counts"]["skipped_not_archivable"],
            1
        );

        let ready = batch_result(&outcome.payload, "ready-to-archive");
        assert_eq!(ready["result"], "applied");
        assert_eq!(ready["inspection"]["session"]["archived"], true);
        assert_eq!(ready["action"]["kind"], "session_archived");
        assert_eq!(
            ready["inspection"]["recent_events"]
                .as_array()
                .expect("ready recent events")
                .last()
                .expect("ready latest event")["event_kind"],
            "session_archived"
        );

        let archived = batch_result(&outcome.payload, "already-archived");
        assert_eq!(archived["result"], "skipped_already_archived");
        assert_eq!(archived["inspection"]["session"]["archived"], true);

        let running = batch_result(&outcome.payload, "running-child");
        assert_eq!(running["result"], "skipped_not_archivable");
        assert_eq!(running["inspection"]["session"]["state"], "running");

        assert!(
            repo.load_session_summary_with_legacy_fallback("ready-to-archive")
                .expect("load ready summary")
                .expect("ready session")
                .archived_at
                .is_some()
        );
        assert!(
            repo.load_session_summary_with_legacy_fallback("already-archived")
                .expect("load archived summary")
                .expect("archived session")
                .archived_at
                .is_some()
        );
        assert_eq!(
            repo.load_session_summary_with_legacy_fallback("running-child")
                .expect("load running summary")
                .expect("running session")
                .archived_at,
            None
        );
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
