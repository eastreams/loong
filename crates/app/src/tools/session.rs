#[cfg(feature = "memory-sqlite")]
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "memory-sqlite")]
use tokio::time::{Duration, Instant, timeout};

use loong_contracts::{
    GovernedSessionMode, GovernedWorkflowPhase, ToolCoreOutcome, ToolCoreRequest,
    WorkflowOperationKind, WorkflowOperationScope, WorktreeBindingDescriptor,
};
use serde_json::{Value, json};

use super::payload::{
    optional_payload_limit, optional_payload_offset, optional_payload_string,
    required_payload_string,
};

use crate::config::{SessionVisibility, ToolConfig};
#[cfg(feature = "memory-sqlite")]
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentHandle,
    ConstrainedSubagentIdentity, ConstrainedSubagentProfile, DelegateBuiltinProfile,
    InterAgentMessage, coordination_actions_for_subagent_handle, mailbox_for_session,
    subagent_surface_fields,
};
#[cfg(feature = "memory-sqlite")]
use crate::runtime_self_continuity;
#[cfg(feature = "memory-sqlite")]
use crate::session::frozen_result::capture_frozen_result;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{
    RECOVERY_EVENT_KIND, RECOVERY_KIND_QUEUED_ASYNC_OVERDUE_MARKED_FAILED,
    RECOVERY_KIND_RUNNING_ASYNC_OVERDUE_MARKED_FAILED, SessionRecoveryRecord,
    build_queued_async_overdue_recovery_payload, build_running_async_overdue_recovery_payload,
    observe_missing_recovery, recovery_json,
};
use crate::session::store::{self, SessionStoreConfig};
#[cfg(feature = "memory-sqlite")]
use crate::session::{
    DELEGATE_CANCEL_REASON_OPERATOR_REQUESTED, DELEGATE_CANCEL_REQUESTED_EVENT_KIND,
    DELEGATE_CANCELLED_EVENT_KIND, delegate_cancelled_error, parse_delegate_cancelled_reason,
};
#[cfg(feature = "memory-sqlite")]
use crate::task_progress::{
    TASK_PROGRESS_EVENT_KIND, TaskProgressRecord, resolve_task_identity_for_event,
    resolve_task_identity_for_session, task_progress_from_event_payload,
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
use crate::{
    config::LoongConfig,
    conversation::{
        ConversationRuntime, ConversationRuntimeBinding,
        run_started_delegate_child_turn_with_runtime,
        with_prepared_subagent_spawn_cleanup_if_kernel_bound,
    },
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
    pub workflow: SessionWorkflowRecord,
    pub subagent_contract: Option<ConstrainedSubagentContractView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionObservationSnapshot {
    pub inspection: SessionInspectionSnapshot,
    pub tail_events: Vec<SessionEventRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DelegateExecutionContract {
    execution: ConstrainedSubagentExecution,
    profile: Option<DelegateBuiltinProfile>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionDelegateLifecycleRecord {
    pub(crate) profile: Option<&'static str>,
    pub(crate) mode: &'static str,
    pub(crate) phase: &'static str,
    pub(crate) queued_at: Option<i64>,
    pub(crate) started_at: Option<i64>,
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) execution: Option<ConstrainedSubagentExecution>,
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
pub(crate) struct SessionWorkflowRecord {
    pub(crate) workflow_id: String,
    pub(crate) task: Option<String>,
    pub(crate) phase: Option<GovernedWorkflowPhase>,
    pub(crate) operation_kind: Option<WorkflowOperationKind>,
    pub(crate) operation_scope: Option<WorkflowOperationScope>,
    pub(crate) task_session_id: Option<String>,
    pub(crate) lineage_root_session_id: Option<String>,
    pub(crate) lineage_depth: Option<usize>,
    pub(crate) task_progress: Option<TaskProgressRecord>,
    pub(crate) runtime_self_continuity: Option<SessionRuntimeSelfContinuityRecord>,
    pub(crate) binding: Option<SessionWorkflowBindingRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionWorkflowBindingRecord {
    pub(crate) session_id: String,
    pub(crate) task_id: String,
    pub(crate) task_session_id: String,
    pub(crate) mode: GovernedSessionMode,
    pub(crate) execution_surface: String,
    pub(crate) worktree: Option<WorktreeBindingDescriptor>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRuntimeSelfContinuityRecord {
    pub(crate) present: bool,
    pub(crate) resolved_identity_present: bool,
    pub(crate) session_profile_projection_present: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionsListRequest {
    limit: usize,
    offset: usize,
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
struct TasksListRequest {
    limit: usize,
    offset: usize,
    task_state: Option<String>,
    stable_only: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TasksSearchRequest {
    query: String,
    max_results: usize,
    task_state: Option<String>,
    stable_only: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionTargetRequest {
    session_ids: Vec<String>,
    legacy_single: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskTargetRequest {
    task_ids: Vec<String>,
    legacy_single: bool,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTaskTarget {
    task_id: String,
    owner_session_id: String,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleTaskRecord {
    task_id: String,
    owner_session_id: String,
    session_label: Option<String>,
    session_updated_at: i64,
    task_progress: TaskProgressRecord,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleTaskSessionRecord {
    task_id: String,
    owner_session_id: String,
    task_session_id: String,
    session_label: Option<String>,
    session_state: SessionState,
    archived: bool,
    lineage_event_id: i64,
    session_updated_at: i64,
    task_progress: Option<TaskProgressRecord>,
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
    config: &SessionStoreConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_session_tool_with_policies(request, current_session_id, config, &ToolConfig::default())
}

pub fn execute_session_tool_with_policies(
    request: ToolCoreRequest,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
            "tasks_list" => execute_tasks_list(payload, current_session_id, config, tool_config),
            "tasks_search" => {
                execute_tasks_search(payload, current_session_id, config, tool_config)
            }
            "task_history" => {
                execute_task_history(payload, current_session_id, config, tool_config)
            }
            "task_events" => execute_task_events(payload, current_session_id, config, tool_config),
            "session_tool_policy_status" => {
                execute_session_tool_policy_status(payload, current_session_id, config, tool_config)
            }
            "session_tool_policy_set" => {
                execute_session_tool_policy_set(payload, current_session_id, config, tool_config)
            }
            "session_tool_policy_clear" => {
                execute_session_tool_policy_clear(payload, current_session_id, config, tool_config)
            }
            "session_search" => super::session_search::execute_session_search_with_policies(
                payload,
                current_session_id,
                config,
                tool_config,
            ),
            "session_status" => {
                execute_session_status(payload, current_session_id, config, tool_config)
            }
            "task_status" => execute_task_status(payload, current_session_id, config, tool_config),
            "session_continue" => Err(
                "app_tool_not_found: session_continue requires the runtime-aware dispatcher"
                    .to_owned(),
            ),
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
fn rewrite_task_payload_aliases(mut payload: Value, task_tool_name: &str) -> Value {
    let top_level_task_id = canonical_task_id_from_value(&payload);
    let top_level_owner_session_id = owner_session_id_from_value(&payload);
    let top_level_task_session_id = task_session_id_from_value(&payload);
    let top_level_task_session_count = task_session_count_from_value(&payload);
    let top_level_task_sessions = task_sessions_from_value(&payload);
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    object.insert("tool".to_owned(), Value::String(task_tool_name.to_owned()));

    if let Some(task_id) = top_level_task_id.map(Value::String) {
        object.insert("task_id".to_owned(), task_id);
    }
    if let Some(owner_session_id) = top_level_owner_session_id.map(Value::String) {
        object.insert("owner_session_id".to_owned(), owner_session_id);
    }
    if let Some(task_session_id) = top_level_task_session_id.map(Value::String) {
        object.insert("task_session_id".to_owned(), task_session_id);
    }
    if let Some(task_session_count) = top_level_task_session_count {
        object.insert(
            "task_session_count".to_owned(),
            Value::from(task_session_count),
        );
    }
    if let Some(task_sessions) = top_level_task_sessions {
        object.insert("task_sessions".to_owned(), Value::Array(task_sessions));
    }

    if let Some(Value::Array(results)) = object.get_mut("results") {
        for result in results {
            let task_id = canonical_task_id_from_value(result);
            let owner_session_id = owner_session_id_from_value(result);
            let task_session_id = task_session_id_from_value(result);
            let task_session_count = task_session_count_from_value(result);
            let task_sessions = task_sessions_from_value(result);
            let task_state = result.get("inspection").and_then(task_state_from_payload);
            let Some(result_object) = result.as_object_mut() else {
                continue;
            };
            if let Some(task_id) = task_id.map(Value::String) {
                result_object.insert("task_id".to_owned(), task_id);
            }
            if let Some(owner_session_id) = owner_session_id.map(Value::String) {
                result_object.insert("owner_session_id".to_owned(), owner_session_id);
            }
            if let Some(task_session_id) = task_session_id.map(Value::String) {
                result_object.insert("task_session_id".to_owned(), task_session_id);
            }
            if let Some(task_session_count) = task_session_count {
                result_object.insert(
                    "task_session_count".to_owned(),
                    Value::from(task_session_count),
                );
            }
            if let Some(task_sessions) = task_sessions {
                result_object.insert("task_sessions".to_owned(), Value::Array(task_sessions));
            }
            if let Some(task_state) = task_state.map(Value::String) {
                let task_is_stable = task_state
                    .as_str()
                    .map(task_state_is_stable)
                    .unwrap_or(false);
                result_object.insert("task_state".to_owned(), task_state);
                result_object.insert("task_is_stable".to_owned(), Value::Bool(task_is_stable));
            }
            result_object.remove("session_id");
        }
    }

    payload
}

#[cfg(feature = "memory-sqlite")]
fn canonical_task_id_from_value(payload: &Value) -> Option<String> {
    payload
        .get("task_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("inspection")
                .and_then(canonical_task_id_from_value)
        })
        .or_else(|| {
            payload
                .get("task_progress")
                .and_then(|value| value.get("task_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .get("workflow")
                .and_then(|value| value.get("task_progress"))
                .and_then(|value| value.get("task_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .get("workflow")
                .and_then(|value| value.get("binding"))
                .and_then(|value| value.get("task_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

#[cfg(feature = "memory-sqlite")]
fn owner_session_id_from_value(payload: &Value) -> Option<String> {
    payload
        .get("owner_session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("inspection")
                .and_then(owner_session_id_from_value)
        })
        .or_else(|| {
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            payload
                .get("session")
                .and_then(|session| session.get("session_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

#[cfg(feature = "memory-sqlite")]
fn task_session_id_from_value(payload: &Value) -> Option<String> {
    payload
        .get("task_session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("inspection")
                .and_then(task_session_id_from_value)
        })
        .or_else(|| {
            payload
                .get("workflow")
                .and_then(|value| value.get("binding"))
                .and_then(|value| value.get("task_session_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

#[cfg(feature = "memory-sqlite")]
fn task_session_count_from_value(payload: &Value) -> Option<u64> {
    payload
        .get("task_session_count")
        .and_then(Value::as_u64)
        .or_else(|| {
            payload
                .get("inspection")
                .and_then(task_session_count_from_value)
        })
}

#[cfg(feature = "memory-sqlite")]
fn task_sessions_from_value(payload: &Value) -> Option<Vec<Value>> {
    payload
        .get("task_sessions")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| payload.get("inspection").and_then(task_sessions_from_value))
}

#[cfg(feature = "memory-sqlite")]
fn task_state_from_payload(payload: &Value) -> Option<String> {
    let inspection_task_state = payload.get("inspection").and_then(task_state_from_payload);
    if inspection_task_state.is_some() {
        return inspection_task_state;
    }

    let terminal_session_state = payload
        .get("session")
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .map(|value| match value {
            "completed" => "completed".to_owned(),
            "failed" | "timed_out" => "failed".to_owned(),
            other => other.to_owned(),
        });
    let task_progress_state = payload
        .get("task_progress")
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("workflow")
                .and_then(|value| value.get("task_progress"))
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });

    match task_progress_state.as_deref() {
        Some("active") | Some("verifying") => terminal_session_state.or(task_progress_state),
        Some(_) => task_progress_state,
        None => terminal_session_state,
    }
}

#[cfg(feature = "memory-sqlite")]
fn task_state_is_stable(state: &str) -> bool {
    matches!(state, "waiting" | "blocked" | "completed" | "failed")
}

#[cfg(feature = "memory-sqlite")]
fn decorate_task_status_payload(mut payload: Value, task_state: Option<String>) -> Value {
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    let task_state = task_state.map(Value::String).unwrap_or(Value::Null);
    let task_is_stable = task_state
        .as_str()
        .map(task_state_is_stable)
        .unwrap_or(false);
    object.insert("task_state".to_owned(), task_state);
    object.insert("task_is_stable".to_owned(), Value::Bool(task_is_stable));
    payload
}

#[cfg(feature = "memory-sqlite")]
fn decorate_task_lineage_payload(
    mut payload: Value,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
) -> Value {
    let current_task_session_id = lineage_records
        .iter()
        .find(|lineage_record| lineage_record.owner_session_id == current_owner_session_id)
        .map(|lineage_record| lineage_record.task_session_id.clone())
        .unwrap_or_else(|| current_owner_session_id.to_owned());
    let task_sessions = lineage_records
        .iter()
        .map(|lineage_record| task_session_summary_json(lineage_record, current_owner_session_id))
        .collect::<Vec<_>>();
    let task_session_count = task_sessions.len();
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    object.insert(
        "task_session_count".to_owned(),
        Value::from(task_session_count as u64),
    );
    object.insert(
        "task_session_id".to_owned(),
        Value::String(current_task_session_id),
    );
    object.insert("task_sessions".to_owned(), Value::Array(task_sessions));
    payload
}

#[cfg(feature = "memory-sqlite")]
fn stable_task_wait_status(snapshot: &SessionInspectionSnapshot) -> Option<&'static str> {
    let terminal_session_status = match snapshot.session.state {
        SessionState::Completed => Some("completed"),
        SessionState::Failed | SessionState::TimedOut => Some("failed"),
        SessionState::Ready | SessionState::Running => None,
    };

    if let Some(task_progress) = snapshot.workflow.task_progress.as_ref() {
        return match task_progress.status {
            crate::task_progress::TaskProgressStatus::Active
            | crate::task_progress::TaskProgressStatus::Verifying => terminal_session_status,
            crate::task_progress::TaskProgressStatus::Waiting => Some("waiting"),
            crate::task_progress::TaskProgressStatus::Blocked => Some("blocked"),
            crate::task_progress::TaskProgressStatus::Completed => Some("completed"),
            crate::task_progress::TaskProgressStatus::Failed => Some("failed"),
        };
    }

    terminal_session_status
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionContinueRequest {
    session_id: String,
    input: String,
    timeout_seconds: u64,
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn continue_session_with_runtime<R: ConversationRuntime + ?Sized>(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    app_config: &LoongConfig,
    runtime: &R,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<ToolCoreOutcome, String> {
    if !tool_config.sessions.enabled {
        return Err("app_tool_disabled: session tools are disabled by config".to_owned());
    }
    if !tool_config.sessions.allow_mutation {
        return Err(
            "app_tool_disabled: session mutation tool `session_continue` is disabled by config"
                .to_owned(),
        );
    }

    let repo = SessionRepository::new(memory_config)?;
    let request = parse_session_continue_request(
        &payload,
        current_session_id,
        memory_config,
        app_config.tools.delegate.timeout_seconds,
    )?;
    ensure_visible(
        &repo,
        current_session_id,
        &request.session_id,
        tool_config.sessions.visibility,
    )?;

    let target_session = repo
        .load_session_summary_with_legacy_fallback(&request.session_id)?
        .ok_or_else(|| format!("session_not_found: `{}`", request.session_id))?;
    if target_session.kind != SessionKind::DelegateChild {
        return Err(format!(
            "session_continue_not_supported: session `{}` is not a delegate child",
            request.session_id
        ));
    }
    if target_session.session_id == current_session_id {
        return Err(
            "session_continue_not_supported: current session cannot continue itself".to_owned(),
        );
    }
    if target_session.state == SessionState::Running {
        return Err(format!(
            "session_continue_busy: session `{}` is already running",
            request.session_id
        ));
    }
    let session_is_completed = target_session.state == SessionState::Completed;
    let session_is_archived = target_session.archived_at.is_some();
    if !session_is_completed || session_is_archived {
        return Err(format!(
            "session_continue_not_supported: session `{}` must be an unarchived completed delegate child",
            request.session_id
        ));
    }

    let parent_session_id = target_session.parent_session_id.clone().ok_or_else(|| {
        format!(
            "session_continue_lineage_missing: session `{}` has no parent session",
            request.session_id
        )
    })?;
    let execution =
        load_delegate_execution_contract(&repo, &request.session_id)?.ok_or_else(|| {
            format!(
                "session_continue_missing_execution_contract: session `{}` has no delegate lifecycle anchor",
                request.session_id
            )
        })?;

    let child_label = target_session.label.clone();
    let expected_state = target_session.state;
    let child_session_id = request.session_id.clone();
    let current_session_id = current_session_id.to_owned();
    let prior_terminal_outcome = repo.load_terminal_outcome(&child_session_id)?;
    let effective_timeout_seconds = request
        .timeout_seconds
        .min(app_config.tools.delegate.timeout_seconds);
    let mut continued_execution = execution.execution.clone();
    continued_execution.timeout_seconds = effective_timeout_seconds;
    with_prepared_subagent_spawn_cleanup_if_kernel_bound(
        runtime,
        &parent_session_id,
        &child_session_id,
        binding,
        || async {
            let transitioned = repo
                .transition_session_with_event_if_current(
                    &child_session_id,
                    crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state,
                    next_state: SessionState::Running,
                    last_error: None,
                    event_kind: "delegate_started".to_owned(),
                    actor_session_id: Some(current_session_id.clone()),
                    event_payload_json: continued_execution.spawn_payload_with_profile(
                        &request.input,
                        child_label.as_deref(),
                        execution.profile,
                        ),
                    },
                )?;
            if transitioned.is_none() {
                return Err(format!(
                    "session_continue_state_changed: session `{}` is no longer continuable from state `{}`",
                    child_session_id,
                    expected_state.as_str()
                ));
            }

            let mut outcome = run_started_delegate_child_turn_with_runtime(
                app_config,
                runtime,
                &child_session_id,
                &parent_session_id,
                child_label.clone(),
                &request.input,
                execution.profile,
                continued_execution,
                effective_timeout_seconds,
                binding,
            )
            .await?;
            if outcome.status != "ok"
                && let Some(prior_terminal_outcome) = prior_terminal_outcome.as_ref()
            {
                repo.upsert_terminal_outcome(
                    &child_session_id,
                    &prior_terminal_outcome.status,
                    prior_terminal_outcome.payload_json.clone(),
                )
                .map_err(|error| {
                    format!(
                        "session_continue_restore_terminal_outcome_failed: {error}"
                    )
                })?;
            }
            inject_session_continue_payload(
                &mut outcome,
                &child_session_id,
                expected_state,
                execution.profile,
                effective_timeout_seconds,
            );
            Ok(outcome)
        },
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
fn inject_session_continue_payload(
    outcome: &mut ToolCoreOutcome,
    session_id: &str,
    previous_state: SessionState,
    profile: Option<DelegateBuiltinProfile>,
    timeout_seconds: u64,
) {
    if let Some(object) = outcome.payload.as_object_mut() {
        object.insert("tool".to_owned(), json!("session_continue"));
        object.insert("session_id".to_owned(), json!(session_id));
        object.insert("previous_state".to_owned(), json!(previous_state.as_str()));
        if let Some(profile) = profile {
            object.insert("profile".to_owned(), json!(profile.as_str()));
        }
        object.insert("timeout_seconds".to_owned(), json!(timeout_seconds));
        object.insert("continued".to_owned(), json!(true));
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_session_continue_request(
    payload: &Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    default_timeout_seconds: u64,
) -> Result<SessionContinueRequest, String> {
    let session_id = required_payload_string(payload, "session_id", "session_continue")?;
    let input = required_payload_string(payload, "input", "session_continue")?;
    let explicit_timeout_seconds = match payload.get("timeout_seconds") {
        Some(value) => {
            let timeout_seconds = value.as_u64().ok_or_else(|| {
                format!("invalid_timeout_seconds: expected a positive integer, got: {value}")
            })?;
            if timeout_seconds == 0 {
                return Err("invalid_timeout_seconds: expected a positive integer".to_owned());
            }
            Some(timeout_seconds)
        }
        None => None,
    };
    let timeout_seconds = explicit_timeout_seconds
        .or_else(|| {
            let repo = SessionRepository::new(memory_config).ok()?;
            load_delegate_execution_contract(&repo, &session_id)
                .ok()
                .flatten()
                .map(|execution| execution.execution.timeout_seconds)
        })
        .unwrap_or(default_timeout_seconds);

    if session_id == current_session_id {
        return Err(
            "session_continue_not_supported: target session_id must differ from current_session_id"
                .to_owned(),
        );
    }

    Ok(SessionContinueRequest {
        session_id,
        input,
        timeout_seconds,
    })
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_execution_contract(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<DelegateExecutionContract>, String> {
    let events = repo.list_delegate_lifecycle_events(session_id)?;
    let mut resolved_execution = None;
    let mut resolved_profile = None;

    for event in events.into_iter().rev() {
        let is_delegate_anchor = matches!(
            event.event_kind.as_str(),
            "delegate_queued" | "delegate_started"
        );
        if !is_delegate_anchor {
            continue;
        }

        if resolved_execution.is_none() {
            resolved_execution =
                ConstrainedSubagentExecution::from_event_payload(&event.payload_json);
        }
        if resolved_profile.is_none() {
            resolved_profile =
                ConstrainedSubagentExecution::profile_from_event_payload(&event.payload_json);
        }
        if resolved_execution.is_some() && resolved_profile.is_some() {
            break;
        }
    }

    Ok(
        resolved_execution.map(|execution| DelegateExecutionContract {
            execution,
            profile: resolved_profile,
        }),
    )
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
pub(super) async fn wait_for_session_tool_with_policies(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
pub(super) async fn wait_for_task_tool_with_policies(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?.to_owned();
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(1_000)
        .clamp(1, 30_000);
    let event_limit = tool_config.sessions.history_limit.min(50);

    wait_for_single_task_with_policies(
        target_task_id.as_str(),
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
async fn wait_for_single_task_with_policies(
    target_task_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let started_at = Instant::now();
    let poll_interval_ms = 100_u64;
    let mut next_after_id = after_id.unwrap_or(0).max(0);
    let mut observed_events = Vec::new();
    let mailbox = mailbox_for_session(current_session_id);
    let mut mailbox_subscription = mailbox.subscribe();

    loop {
        let repo = SessionRepository::new(config)?;
        let resolved_target =
            resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
        let lineage_records =
            load_task_lineage_records(&repo, current_session_id, &resolved_target)?;
        let owner_session_id = resolved_target.owner_session_id.clone();
        let observation = observe_visible_session_with_policies(
            owner_session_id.as_str(),
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

        if let Some(wait_status) = stable_task_wait_status(&snapshot) {
            return Ok(task_wait_outcome(
                "ok",
                snapshot,
                lineage_records.as_slice(),
                owner_session_id.as_str(),
                wait_status,
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
                payload: task_wait_payload(
                    snapshot,
                    lineage_records.as_slice(),
                    owner_session_id.as_str(),
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
        let wait_window_ms = remaining_ms.min(poll_interval_ms);
        let drained: Vec<InterAgentMessage> = mailbox.drain().await;
        if !drained.is_empty() {
            continue;
        }

        let wait_result = timeout(
            Duration::from_millis(wait_window_ms),
            mailbox_subscription.changed(),
        )
        .await;
        if let Ok(Err(_)) = wait_result {
            return Err("task_wait_internal_error: mailbox subscription closed".to_owned());
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn execute_sessions_list(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
        let delegate_events = if session.kind == SessionKind::DelegateChild {
            Some(load_delegate_lifecycle_events(&repo, &session)?)
        } else {
            None
        };
        let delegate_lifecycle = delegate_events
            .as_deref()
            .and_then(|events| session_delegate_lifecycle_at(&session, events, now_ts));
        let subagent_contract = resolve_subagent_contract_for_session(
            &repo,
            &session,
            delegate_lifecycle.as_ref(),
            tool_config,
        )?;
        if request.overdue_only
            && !delegate_lifecycle
                .as_ref()
                .and_then(|lifecycle| lifecycle.staleness.as_ref())
                .map(|staleness| staleness.state == "overdue")
                .unwrap_or(false)
        {
            continue;
        }
        listed_sessions.push((
            session,
            delegate_events,
            delegate_lifecycle,
            subagent_contract,
        ));
    }

    let matched_count = listed_sessions.len();
    let effective_offset = request.offset.min(matched_count);
    let page_limit = request.limit.saturating_add(1);
    let visible_sessions = listed_sessions.into_iter();
    let offset_sessions = visible_sessions.skip(effective_offset);
    let bounded_sessions = offset_sessions.take(page_limit);
    let mut listed_sessions = bounded_sessions.collect::<Vec<_>>();
    let has_more = listed_sessions.len() > request.limit;
    if has_more {
        let _ = listed_sessions.pop();
    }
    let returned_count = listed_sessions.len();
    let mut session_payloads = Vec::with_capacity(returned_count);
    for (session, delegate_events, delegate_lifecycle, subagent_contract) in listed_sessions {
        let workflow = load_session_workflow_record(&repo, &session, delegate_events.as_deref())?;
        let payload = session_summary_json_with_delegate_lifecycle(
            session,
            workflow,
            delegate_lifecycle,
            subagent_contract,
            include_delegate_lifecycle,
        );
        session_payloads.push(payload);
    }
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "current_session_id": current_session_id,
            "filters": sessions_list_filters_json(&request),
            "matched_count": matched_count,
            "returned_count": returned_count,
            "has_more": has_more,
            "sessions": session_payloads,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_tasks_list(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_tasks_list_request(&payload, tool_config);
    let visible_tasks = load_visible_task_records(&repo, current_session_id)?;

    let mut tasks = Vec::new();
    for visible_task in visible_tasks {
        let task_progress = visible_task.task_progress;
        let task_state = task_progress.status.as_str().to_owned();
        let task_is_stable = task_progress.status.is_stable();
        let state_filter = request.task_state.as_deref();
        let matches_state = state_filter.is_none_or(|expected| expected == task_state.as_str());
        if !matches_state {
            continue;
        }
        if request.stable_only && !task_is_stable {
            continue;
        }

        let verification_state = task_progress
            .verification_state
            .map(|value| value.as_str().to_owned());
        tasks.push(json!({
            "task_id": visible_task.task_id,
            "task_state": task_state,
            "task_is_stable": task_is_stable,
            "intent_summary": task_progress.intent_summary,
            "verification_state": verification_state,
            "owner_session_id": visible_task.owner_session_id,
            "session_label": visible_task.session_label,
            "updated_at": task_progress.updated_at,
            "active_handles": task_progress.active_handles,
            "resume_recipe": task_progress.resume_recipe,
        }));
    }

    let matched_count = tasks.len();
    let effective_offset = request.offset.min(matched_count);
    let mut tasks = tasks
        .into_iter()
        .skip(effective_offset)
        .take(request.limit.saturating_add(1))
        .collect::<Vec<_>>();
    let has_more = tasks.len() > request.limit;
    if has_more {
        let _ = tasks.pop();
    }
    let returned_count = tasks.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "tasks_list",
            "current_session_id": current_session_id,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "has_more": has_more,
            "filters": {
                "task_state": request.task_state,
                "stable_only": request.stable_only,
                "limit": request.limit,
                "offset": request.offset,
            },
            "tasks": tasks,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_tasks_search(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_tasks_search_request(&payload, tool_config)?;
    let visible_tasks = load_visible_task_records(&repo, current_session_id)?;
    let query = request.query.to_ascii_lowercase();

    let mut tasks = Vec::new();
    for visible_task in visible_tasks {
        let task_progress = visible_task.task_progress;
        let task_state = task_progress.status.as_str().to_owned();
        let task_is_stable = task_progress.status.is_stable();
        let state_filter = request.task_state.as_deref();
        let matches_state = state_filter.is_none_or(|expected| expected == task_state.as_str());
        if !matches_state {
            continue;
        }
        if request.stable_only && !task_is_stable {
            continue;
        }

        let session_label = visible_task.session_label.as_deref().unwrap_or_default();
        let haystack = [
            visible_task.task_id.as_str(),
            visible_task.owner_session_id.as_str(),
            task_state.as_str(),
            task_progress.intent_summary.as_deref().unwrap_or_default(),
            session_label,
            task_progress.owner_kind.as_str(),
        ]
        .join(" ")
        .to_ascii_lowercase();

        if !haystack.contains(query.as_str()) {
            continue;
        }

        let verification_state = task_progress
            .verification_state
            .map(|value| value.as_str().to_owned());
        tasks.push(json!({
            "task_id": visible_task.task_id,
            "task_state": task_state,
            "task_is_stable": task_is_stable,
            "intent_summary": task_progress.intent_summary,
            "verification_state": verification_state,
            "owner_session_id": visible_task.owner_session_id,
            "session_label": visible_task.session_label,
            "updated_at": task_progress.updated_at,
            "active_handles": task_progress.active_handles,
            "resume_recipe": task_progress.resume_recipe,
        }));
    }

    let matched_count = tasks.len();
    let tasks = tasks
        .into_iter()
        .take(request.max_results)
        .collect::<Vec<_>>();
    let returned_count = tasks.len();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "tasks_search",
            "current_session_id": current_session_id,
            "query": request.query,
            "matched_count": matched_count,
            "returned_count": returned_count,
            "filters": {
                "task_state": request.task_state,
                "stable_only": request.stable_only,
                "max_results": request.max_results,
            },
            "tasks": tasks,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_events(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    let turns = store::window_session_turns(&target_session_id, limit, config)
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
fn execute_task_history(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let lineage_records = load_task_lineage_records(&repo, current_session_id, &resolved_target)?;
    let current_owner_session_id = resolved_target.owner_session_id.as_str();
    let current_task_session_id = lineage_records
        .iter()
        .find(|lineage_record| lineage_record.owner_session_id == current_owner_session_id)
        .map(|lineage_record| lineage_record.task_session_id.clone())
        .unwrap_or_else(|| resolved_target.owner_session_id.clone());
    let turns = load_task_history_turns(
        config,
        lineage_records.as_slice(),
        current_owner_session_id,
        limit,
    )?;
    let task_events = load_task_history_events(
        &repo,
        lineage_records.as_slice(),
        current_owner_session_id,
        None,
        limit,
    )?;
    let task_sessions = lineage_records
        .iter()
        .map(|lineage_record| task_session_summary_json(lineage_record, current_owner_session_id))
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "task_history",
            "task_id": resolved_target.task_id,
            "owner_session_id": resolved_target.owner_session_id,
            "task_session_id": current_task_session_id,
            "lineage_session_count": lineage_records.len(),
            "limit": limit,
            "task_sessions": task_sessions,
            "turns": turns,
            "task_events": task_events,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_task_events(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = parse_task_target_request(&payload, "task_id", None)?;
    let target_task_id = legacy_single_task_id(&request.task_ids)?;
    let after_id = payload.get("after_id").and_then(Value::as_i64);
    let repo = SessionRepository::new(config)?;
    let resolved_target =
        resolve_task_target(&repo, current_session_id, target_task_id, tool_config)?;
    let default_limit = tool_config.sessions.history_limit.min(50);
    let limit = optional_payload_limit(
        &payload,
        "limit",
        default_limit,
        tool_config.sessions.history_limit,
    );
    let lineage_records = load_task_lineage_records(&repo, current_session_id, &resolved_target)?;
    let current_owner_session_id = resolved_target.owner_session_id.as_str();
    let current_task_session_id = lineage_records
        .iter()
        .find(|lineage_record| lineage_record.owner_session_id == current_owner_session_id)
        .map(|lineage_record| lineage_record.task_session_id.clone())
        .unwrap_or_else(|| resolved_target.owner_session_id.clone());
    let (events, next_after_id) = load_task_event_window(
        &repo,
        lineage_records.as_slice(),
        current_owner_session_id,
        after_id,
        limit,
    )?;
    let task_sessions = lineage_records
        .iter()
        .map(|lineage_record| task_session_summary_json(lineage_record, current_owner_session_id))
        .collect::<Vec<_>>();

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "tool": "task_events",
            "task_id": resolved_target.task_id,
            "owner_session_id": resolved_target.owner_session_id,
            "task_session_id": current_task_session_id,
            "task_session_count": lineage_records.len(),
            "after_id": after_id,
            "next_after_id": next_after_id,
            "limit": limit,
            "task_sessions": task_sessions,
            "events": events,
        }),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_tool_policy_status(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
fn execute_task_status(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let request = parse_task_target_request(&payload, "task_id", Some("task_ids"))?;
    let resolved_targets =
        resolve_task_targets(&repo, current_session_id, &request.task_ids, tool_config)?;

    if request.legacy_single {
        let resolved_target = legacy_single_task_target(&resolved_targets)?;
        let snapshot = inspect_visible_session_with_policies(
            &resolved_target.owner_session_id,
            current_session_id,
            config,
            tool_config,
            5,
        )?;
        let lineage_records =
            load_task_lineage_records(&repo, current_session_id, resolved_target)?;
        let payload = session_inspection_payload(snapshot);
        let task_state = task_state_from_payload(&payload);
        let payload = rewrite_task_payload_aliases(payload, "task_status");
        let payload = decorate_task_status_payload(payload, task_state);
        let payload = decorate_task_lineage_payload(
            payload,
            lineage_records.as_slice(),
            &resolved_target.owner_session_id,
        );

        return Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload,
        });
    }

    let mut results = Vec::with_capacity(resolved_targets.len());
    for resolved_target in &resolved_targets {
        results.push(execute_task_status_batch_result(
            resolved_target,
            current_session_id,
            config,
            tool_config,
        )?);
    }

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: rewrite_task_payload_aliases(
            session_batch_payload_without_dry_run(
                "task_status",
                current_session_id,
                resolved_targets.len(),
                results,
            ),
            "task_status",
        ),
    })
}

#[cfg(feature = "memory-sqlite")]
fn execute_task_status_batch_result(
    resolved_target: &ResolvedTaskTarget,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<SessionBatchResultRecord, String> {
    let repo = SessionRepository::new(config)?;
    let owner_session_id = resolved_target.owner_session_id.as_str();
    if let Err(error) = ensure_visible(
        &repo,
        current_session_id,
        owner_session_id,
        tool_config.sessions.visibility,
    ) {
        return Ok(session_batch_result(
            resolved_target.owner_session_id.clone(),
            "skipped_not_visible",
            Some(error),
            None,
            None,
        ));
    }

    let snapshot = match inspect_visible_session_with_policies(
        owner_session_id,
        current_session_id,
        config,
        tool_config,
        5,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) if is_session_visibility_skip_error(&error) => {
            return Ok(session_batch_result(
                resolved_target.owner_session_id.clone(),
                "skipped_not_visible",
                Some(error),
                None,
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    let lineage_records = load_task_lineage_records(&repo, current_session_id, resolved_target)?;
    let payload = session_inspection_payload(snapshot);
    let task_state = task_state_from_payload(&payload);
    let payload = rewrite_task_payload_aliases(payload, "task_status");
    let payload = decorate_task_status_payload(payload, task_state);
    let payload =
        decorate_task_lineage_payload(payload, lineage_records.as_slice(), owner_session_id);

    Ok(session_batch_result(
        resolved_target.owner_session_id.clone(),
        "ok",
        None,
        None,
        Some(payload),
    ))
}

#[cfg(feature = "memory-sqlite")]
fn execute_session_recover(
    payload: Value,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let started_at = Instant::now();
    let mut next_after_id = after_id.unwrap_or(0).max(0);
    let mut observed_events = Vec::new();
    let mailbox = mailbox_for_session(current_session_id);
    let mut mailbox_subscription = mailbox.subscribe();

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
        let drained: Vec<InterAgentMessage> = mailbox.drain().await;
        if !drained.is_empty() {
            continue;
        }

        let wait_result = timeout(
            Duration::from_millis(remaining_ms),
            mailbox_subscription.changed(),
        )
        .await;
        if let Ok(Err(_)) = wait_result {
            return Err("session_wait_internal_error: mailbox subscription closed".to_owned());
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[allow(dead_code)]
async fn wait_for_session_batch_with_policies(
    target_session_ids: Vec<String>,
    current_session_id: &str,
    config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    after_id: Option<i64>,
    timeout_ms: u64,
    event_limit: usize,
) -> Result<ToolCoreOutcome, String> {
    let repo = SessionRepository::new(config)?;
    let mailbox = mailbox_for_session(current_session_id);
    let mut mailbox_subscription = mailbox.subscribe();
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
        let drained: Vec<InterAgentMessage> = mailbox.drain().await;
        if !drained.is_empty() {
            continue;
        }

        let wait_result = timeout(
            Duration::from_millis(remaining_ms),
            mailbox_subscription.changed(),
        )
        .await;
        if let Ok(Err(_)) = wait_result {
            return Err("session_wait_internal_error: mailbox subscription closed".to_owned());
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn observe_visible_session_with_policies(
    target_session_id: &str,
    current_session_id: &str,
    config: &SessionStoreConfig,
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
    let workflow = load_session_workflow_record(&repo, &session, Some(delegate_events.as_slice()))?;
    let delegate_lifecycle =
        session_delegate_lifecycle_at(&session, delegate_events.as_slice(), current_unix_ts());
    let subagent_contract = resolve_subagent_contract_for_session(
        &repo,
        &session,
        delegate_lifecycle.as_ref(),
        tool_config,
    )?;

    Ok(SessionObservationSnapshot {
        inspection: SessionInspectionSnapshot {
            session,
            terminal_outcome,
            recent_events,
            delegate_events,
            workflow,
            subagent_contract,
        },
        tail_events,
    })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_subagent_contract_for_session(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
    tool_config: &ToolConfig,
) -> Result<Option<ConstrainedSubagentContractView>, String> {
    if session.kind != SessionKind::DelegateChild {
        return Ok(None);
    }

    if let Some(contract) =
        delegate_lifecycle.and_then(resolve_subagent_contract_from_delegate_lifecycle)
    {
        return Ok(Some(attach_session_label_identity(contract, session)));
    }

    if session.parent_session_id.is_none() {
        return Ok(None);
    }

    let depth = match repo.session_lineage_depth(&session.session_id) {
        Ok(depth) => depth,
        Err(error) if is_expected_lineage_gap_error(&error) => {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "compute session lineage depth for subagent profile failed: {error}"
            ));
        }
    };

    Ok(Some(attach_session_label_identity(
        ConstrainedSubagentContractView::from_profile(ConstrainedSubagentProfile::for_child_depth(
            depth,
            tool_config.delegate.max_depth,
        )),
        session,
    )))
}

#[cfg(feature = "memory-sqlite")]
fn attach_session_label_identity(
    mut contract: ConstrainedSubagentContractView,
    session: &SessionSummaryRecord,
) -> ConstrainedSubagentContractView {
    if contract.identity.is_none() {
        let nickname = session.label.clone();
        if nickname.is_some() {
            contract = contract.with_identity(ConstrainedSubagentIdentity {
                nickname,
                specialization: None,
            });
        }
    }
    contract
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
pub(crate) fn load_session_workflow_record(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_events: Option<&[SessionEventRecord]>,
) -> Result<SessionWorkflowRecord, String> {
    let lineage_root_session_id =
        optional_lineage_lookup(repo.lineage_root_session_id(&session.session_id))?.flatten();
    let lineage_depth = optional_lineage_lookup(repo.session_lineage_depth(&session.session_id))?;

    let loaded_delegate_events = match delegate_events {
        Some(_) => None,
        None if session.kind == SessionKind::DelegateChild => {
            Some(repo.list_delegate_lifecycle_events(&session.session_id)?)
        }
        None => None,
    };
    let delegate_events = match delegate_events {
        Some(events) => events,
        None => loaded_delegate_events.as_deref().unwrap_or(&[]),
    };
    let workflow_id = session_workflow_id(session, lineage_root_session_id.as_deref());
    let task = delegate_events
        .iter()
        .rev()
        .find_map(session_workflow_task_from_event);
    let phase = session_workflow_phase(session, delegate_events);
    let operation_kind = session_workflow_operation_kind(session);
    let operation_scope = session_workflow_operation_scope(session);
    let task_session_id = session_workflow_task_session_id(session);
    let task_progress = repo
        .load_latest_event_by_kind(&session.session_id, TASK_PROGRESS_EVENT_KIND)?
        .as_ref()
        .and_then(|event| task_progress_from_event_payload(&event.payload_json));
    let resolved_task_identity = if session.kind == SessionKind::DelegateChild {
        Some(resolve_task_identity_for_session(repo, &session.session_id))
    } else {
        None
    };
    let runtime_self_continuity =
        load_session_runtime_self_continuity_record(repo, session, delegate_events)?;
    let binding =
        session_workflow_binding_record(session, delegate_events, resolved_task_identity.as_ref());

    Ok(SessionWorkflowRecord {
        workflow_id,
        task,
        phase,
        operation_kind,
        operation_scope,
        task_session_id,
        lineage_root_session_id,
        lineage_depth,
        task_progress,
        runtime_self_continuity,
        binding,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_id(
    session: &SessionSummaryRecord,
    lineage_root_session_id: Option<&str>,
) -> String {
    let fallback_session_id = session.session_id.as_str();
    let resolved_workflow_id = lineage_root_session_id.unwrap_or(fallback_session_id);
    resolved_workflow_id.to_owned()
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_task_from_event(event: &SessionEventRecord) -> Option<String> {
    event
        .payload_json
        .get("task")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_phase(
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
) -> Option<GovernedWorkflowPhase> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let cancellation_reason = session
        .last_error
        .as_deref()
        .and_then(parse_delegate_cancelled_reason);
    let was_cancelled = cancellation_reason.is_some();
    if was_cancelled {
        return Some(GovernedWorkflowPhase::Cancelled);
    }

    let has_delegate_events = !delegate_events.is_empty();
    if !has_delegate_events && session.parent_session_id.is_none() {
        return None;
    }

    match session.state {
        SessionState::Ready => Some(GovernedWorkflowPhase::Execute),
        SessionState::Running => Some(GovernedWorkflowPhase::Execute),
        SessionState::Completed => Some(GovernedWorkflowPhase::Complete),
        SessionState::Failed => Some(GovernedWorkflowPhase::Failed),
        SessionState::TimedOut => Some(GovernedWorkflowPhase::Failed),
    }
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_operation_kind(
    session: &SessionSummaryRecord,
) -> Option<WorkflowOperationKind> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(WorkflowOperationKind::Task)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_operation_scope(
    session: &SessionSummaryRecord,
) -> Option<WorkflowOperationScope> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(WorkflowOperationScope::Task)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_task_session_id(session: &SessionSummaryRecord) -> Option<String> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    Some(session.session_id.clone())
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_record(
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
    resolved_task_identity: Option<&crate::task_progress::ResolvedTaskIdentity>,
) -> Option<SessionWorkflowBindingRecord> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let (execution, execution_surface) =
        latest_delegate_execution_binding_components(delegate_events)?;
    let mode = session_workflow_binding_mode(&execution);
    let worktree = session_workflow_worktree_binding(session, &execution);
    let task_id = resolved_task_identity
        .map(|task_identity| task_identity.task_id.clone())
        .unwrap_or_else(|| session.session_id.clone());
    let task_session_id = resolved_task_identity
        .map(|task_identity| task_identity.task_session_id.clone())
        .unwrap_or_else(|| session.session_id.clone());
    let binding = SessionWorkflowBindingRecord {
        session_id: session.session_id.clone(),
        task_id,
        task_session_id,
        mode,
        execution_surface,
        worktree,
    };

    Some(binding)
}

#[cfg(feature = "memory-sqlite")]
fn latest_delegate_execution_binding_components(
    delegate_events: &[SessionEventRecord],
) -> Option<(ConstrainedSubagentExecution, String)> {
    for event in delegate_events.iter().rev() {
        let event_kind = event.event_kind.as_str();
        let is_delegate_execution_event =
            matches!(event_kind, "delegate_queued" | "delegate_started");
        if !is_delegate_execution_event {
            continue;
        }

        let execution = ConstrainedSubagentExecution::from_event_payload(&event.payload_json)?;
        let execution_surface = session_workflow_execution_surface(event, &execution);
        return Some((execution, execution_surface));
    }

    None
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_execution_surface(
    event: &SessionEventRecord,
    execution: &ConstrainedSubagentExecution,
) -> String {
    let trust_event = crate::trust::extract_trust_event_payload(&event.payload_json);
    if let Some(trust_event) = trust_event {
        return trust_event.source_surface;
    }

    let fallback_surface = match execution.mode {
        crate::conversation::ConstrainedSubagentMode::Inline => "delegate.inline",
        crate::conversation::ConstrainedSubagentMode::Async => "delegate.async",
    };

    fallback_surface.to_owned()
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_mode(execution: &ConstrainedSubagentExecution) -> GovernedSessionMode {
    if execution.kernel_bound {
        return GovernedSessionMode::MutatingCapable;
    }

    GovernedSessionMode::AdvisoryOnly
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_worktree_binding(
    session: &SessionSummaryRecord,
    execution: &ConstrainedSubagentExecution,
) -> Option<WorktreeBindingDescriptor> {
    let workspace_root = execution.workspace_root.as_ref()?;
    let workspace_root_string = workspace_root.display().to_string();
    let worktree = WorktreeBindingDescriptor {
        worktree_id: session.session_id.clone(),
        workspace_root: workspace_root_string,
    };

    Some(worktree)
}

#[cfg(feature = "memory-sqlite")]
fn optional_lineage_lookup<T>(result: Result<T, String>) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_expected_lineage_gap_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "memory-sqlite")]
fn is_expected_lineage_gap_error(error: &str) -> bool {
    let is_broken = error.starts_with("session_lineage_broken:");
    let is_cycle = error.starts_with("session_lineage_cycle_detected:");
    is_broken || is_cycle
}

#[cfg(feature = "memory-sqlite")]
fn load_session_runtime_self_continuity_record(
    repo: &SessionRepository,
    session: &SessionSummaryRecord,
    delegate_events: &[SessionEventRecord],
) -> Result<Option<SessionRuntimeSelfContinuityRecord>, String> {
    let continuity =
        runtime_self_continuity::load_persisted_runtime_self_continuity_with_delegate_events(
            repo,
            &session.session_id,
            Some(delegate_events),
        )?;
    let record = continuity
        .as_ref()
        .map(session_runtime_self_continuity_record_from_continuity);
    Ok(record)
}

#[cfg(feature = "memory-sqlite")]
fn session_runtime_self_continuity_record_from_continuity(
    continuity: &runtime_self_continuity::RuntimeSelfContinuity,
) -> SessionRuntimeSelfContinuityRecord {
    let session_profile_projection_present = continuity
        .session_profile_projection
        .as_deref()
        .is_some_and(|projection| !projection.trim().is_empty());
    SessionRuntimeSelfContinuityRecord {
        present: continuity.has_prompt_projection(),
        resolved_identity_present: continuity.resolved_identity.is_some(),
        session_profile_projection_present,
    }
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
    let diagnostics =
        session_diagnostics_json(&snapshot, terminal_outcome_state, recovery.as_ref());
    let subagent_handle = subagent_handle_for_session(
        &snapshot.session,
        snapshot.subagent_contract.as_ref(),
        delegate_lifecycle.as_ref(),
    );
    let mut payload = json!({
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
            "turn_count": snapshot.session.turn_count,
            "last_turn_at": snapshot.session.last_turn_at,
            "last_error": snapshot.session.last_error,
        },
        "task_progress": snapshot.workflow.task_progress,
        "workflow": session_workflow_json(snapshot.workflow),
        "terminal_outcome_state": terminal_outcome_state,
        "terminal_outcome_missing_reason": terminal_outcome_missing_reason,
        "diagnostics": diagnostics,
        "delegate_lifecycle": delegate_lifecycle
            .map(|lifecycle| session_delegate_lifecycle_json(
                lifecycle,
                snapshot.subagent_contract.as_ref(),
            )),
        "recovery": recovery.map(recovery_json),
        "terminal_outcome": snapshot.terminal_outcome.map(session_terminal_outcome_json),
        "recent_events": snapshot
            .recent_events
            .into_iter()
            .map(session_event_json)
            .collect::<Vec<_>>(),
    });
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    insert_subagent_surface_fields(
        object,
        snapshot.subagent_contract.as_ref(),
        subagent_handle.as_ref(),
    );
    payload
}

#[cfg(feature = "memory-sqlite")]
fn session_diagnostics_json(
    snapshot: &SessionInspectionSnapshot,
    terminal_outcome_state: &str,
    recovery: Option<&SessionRecoveryRecord>,
) -> Value {
    let recent_events = snapshot.recent_events.as_slice();
    let latest_provider_failover = latest_provider_failover_diagnostic(recent_events);
    let recommended_action = recommended_session_action(snapshot);
    let attention_hints = build_session_attention_hints(
        latest_provider_failover.as_ref(),
        recommended_action.as_ref(),
        recovery,
        terminal_outcome_state,
    );

    json!({
        "latest_provider_failover": latest_provider_failover,
        "recommended_action": recommended_action,
        "attention_hints": attention_hints,
    })
}

#[cfg(feature = "memory-sqlite")]
fn latest_provider_failover_diagnostic(recent_events: &[SessionEventRecord]) -> Option<Value> {
    let matching_event = recent_events
        .iter()
        .filter(|event| event.event_kind == "trust_provider_failover")
        .max_by_key(|event| event.ts)?;
    let payload_object = matching_event.payload_json.as_object()?;
    let provider_failover = payload_object.get("provider_failover")?.as_object()?;

    let provider_id = payload_object
        .get("provider_id")
        .cloned()
        .unwrap_or(Value::Null);
    let binding = payload_object
        .get("binding")
        .cloned()
        .unwrap_or(Value::Null);
    let reason = provider_failover
        .get("reason")
        .cloned()
        .unwrap_or(Value::Null);
    let stage = provider_failover
        .get("stage")
        .cloned()
        .unwrap_or(Value::Null);
    let model = provider_failover
        .get("model")
        .cloned()
        .unwrap_or(Value::Null);
    let attempt = provider_failover
        .get("attempt")
        .cloned()
        .unwrap_or(Value::Null);
    let max_attempts = provider_failover
        .get("max_attempts")
        .cloned()
        .unwrap_or(Value::Null);
    let status_code = provider_failover
        .get("status_code")
        .cloned()
        .unwrap_or(Value::Null);
    let request_id = provider_failover
        .get("request_id")
        .cloned()
        .unwrap_or(Value::Null);
    let cf_ray = provider_failover
        .get("cf_ray")
        .cloned()
        .unwrap_or(Value::Null);
    let auth_error = provider_failover
        .get("auth_error")
        .cloned()
        .unwrap_or(Value::Null);
    let auth_error_code = provider_failover
        .get("auth_error_code")
        .cloned()
        .unwrap_or(Value::Null);

    Some(json!({
        "event_id": matching_event.id,
        "event_kind": matching_event.event_kind,
        "ts": matching_event.ts,
        "provider_id": provider_id,
        "binding": binding,
        "reason": reason,
        "stage": stage,
        "model": model,
        "attempt": attempt,
        "max_attempts": max_attempts,
        "status_code": status_code,
        "request_id": request_id,
        "cf_ray": cf_ray,
        "auth_error": auth_error,
        "auth_error_code": auth_error_code,
    }))
}

#[cfg(feature = "memory-sqlite")]
fn recommended_session_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let recover_action = recommended_recover_action(snapshot);
    if recover_action.is_some() {
        return recover_action;
    }

    recommended_resume_action(snapshot)
}

#[cfg(feature = "memory-sqlite")]
fn recommended_recover_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let recover_plan = build_session_recover_plan(snapshot, current_unix_ts()).ok()?;
    let mut recover_action = session_recovery_action_json(&recover_plan);
    let action_object = recover_action.as_object_mut()?;
    action_object.insert(
        "source".to_owned(),
        Value::String("session_recover_plan".to_owned()),
    );
    action_object.insert(
        "tool_name".to_owned(),
        Value::String("session_recover".to_owned()),
    );
    action_object.insert("requires_mutation".to_owned(), Value::Bool(true));
    Some(recover_action)
}

#[cfg(feature = "memory-sqlite")]
fn recommended_resume_action(snapshot: &SessionInspectionSnapshot) -> Option<Value> {
    let task_progress = snapshot.workflow.task_progress.as_ref()?;
    let resume_recipe = task_progress.resume_recipe.as_ref()?;
    let tool_name = resume_recipe.recommended_tool.clone();
    let session_id = resume_recipe.session_id.clone();
    let note = resume_recipe.note.clone();
    let requires_mutation = session_action_requires_mutation(tool_name.as_str());
    let task_status = task_progress.status.as_str().to_owned();

    Some(json!({
        "source": "task_progress_resume_recipe",
        "kind": "follow_resume_recipe",
        "tool_name": tool_name,
        "session_id": session_id,
        "note": note,
        "task_status": task_status,
        "requires_mutation": requires_mutation,
    }))
}

#[cfg(feature = "memory-sqlite")]
fn session_action_requires_mutation(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "session_archive" | "session_cancel" | "session_continue" | "session_recover"
    )
}

#[cfg(feature = "memory-sqlite")]
fn build_session_attention_hints(
    latest_provider_failover: Option<&Value>,
    recommended_action: Option<&Value>,
    recovery: Option<&SessionRecoveryRecord>,
    terminal_outcome_state: &str,
) -> Vec<String> {
    let mut hints = Vec::new();

    if let Some(provider_failover) = latest_provider_failover {
        let reason = provider_failover
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let model = provider_failover
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let stage = provider_failover
            .get("stage")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let request_id = provider_failover
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let auth_error_code = provider_failover
            .get("auth_error_code")
            .and_then(Value::as_str)
            .unwrap_or("-");
        hints.push(format!(
            "provider_failover_present reason={reason} model={model} stage={stage} request_id={request_id} auth_error_code={auth_error_code}"
        ));
    }

    if let Some(action) = recommended_action {
        let tool_name = action
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let kind = action
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let source = action
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        hints.push(format!(
            "recommended_action tool={tool_name} kind={kind} source={source}"
        ));
    }

    if terminal_outcome_state == "missing" {
        let recovery_kind = recovery
            .map(|record| record.kind.as_str())
            .unwrap_or("unknown");
        let recovery_source = recovery
            .map(|record| record.source.as_str())
            .unwrap_or("none");
        hints.push(format!(
            "terminal_outcome_missing kind={recovery_kind} source={recovery_source}"
        ));
    }

    hints
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
    let frozen_result = capture_frozen_result(&outcome, tool_config.delegate.max_frozen_bytes);
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
            frozen_result: Some(frozen_result),
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
    config: &SessionStoreConfig,
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
    config: &SessionStoreConfig,
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
            let frozen_result =
                capture_frozen_result(&outcome, tool_config.delegate.max_frozen_bytes);
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
                    frozen_result: Some(frozen_result),
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
pub(crate) fn session_delegate_lifecycle_at(
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
    let mut profile = None;
    let mut cancellation = None;
    for event in recent_events {
        match event.event_kind.as_str() {
            "delegate_queued" => {
                queued_at = Some(event.ts);
                let parsed_profile =
                    ConstrainedSubagentExecution::profile_from_event_payload(&event.payload_json);
                let parsed_execution =
                    ConstrainedSubagentExecution::from_event_payload(&event.payload_json);
                profile = parsed_profile.or(profile);
                execution = parsed_execution.or(execution);
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
                let parsed_profile =
                    ConstrainedSubagentExecution::profile_from_event_payload(&event.payload_json);
                let parsed_execution =
                    ConstrainedSubagentExecution::from_event_payload(&event.payload_json);
                profile = parsed_profile.or(profile);
                execution = parsed_execution.or(execution);
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
        profile: profile.map(DelegateBuiltinProfile::as_str),
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
fn session_delegate_lifecycle_json(
    lifecycle: SessionDelegateLifecycleRecord,
    subagent_contract: Option<&ConstrainedSubagentContractView>,
) -> Value {
    json!({
        "profile": lifecycle.profile,
        "mode": lifecycle.mode,
        "phase": lifecycle.phase,
        "queued_at": lifecycle.queued_at,
        "started_at": lifecycle.started_at,
        "timeout_seconds": lifecycle.timeout_seconds,
        "contract": subagent_contract.cloned(),
        "execution": lifecycle
            .execution
            .map(ConstrainedSubagentExecution::with_resolved_profile),
        "staleness": lifecycle.staleness.map(session_delegate_staleness_json),
        "cancellation": lifecycle
            .cancellation
            .map(session_delegate_cancellation_json),
    })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_subagent_contract_from_delegate_lifecycle(
    lifecycle: &SessionDelegateLifecycleRecord,
) -> Option<ConstrainedSubagentContractView> {
    lifecycle
        .execution
        .as_ref()
        .map(ConstrainedSubagentExecution::contract_view)
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
    super::runtime_config::merge_runtime_narrowing_sources(
        delegate_runtime_narrowing,
        policy_runtime_narrowing,
    )
}

#[cfg(feature = "memory-sqlite")]
fn tool_view_names(tool_view: &ToolView) -> Vec<String> {
    tool_view.tool_names().map(str::to_owned).collect()
}

#[cfg(feature = "memory-sqlite")]
fn visible_tool_id_names(tool_ids: &[String]) -> Vec<String> {
    let mut visible_tool_ids = Vec::new();

    for tool_id in tool_ids {
        let visible_tool_id = crate::tools::model_visible_tool_name(tool_id.as_str());
        if !visible_tool_ids.contains(&visible_tool_id) {
            visible_tool_ids.push(visible_tool_id);
        }
    }

    visible_tool_ids
}

#[cfg(feature = "memory-sqlite")]
fn runtime_narrowing_json(runtime_narrowing: Option<ToolRuntimeNarrowing>) -> Value {
    match runtime_narrowing {
        Some(runtime_narrowing) => serde_json::to_value(runtime_narrowing).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn build_session_tool_policy_status_payload(
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
    let base_tool_ids = tool_view_names(&base_tool_view);
    let effective_tool_ids = tool_view_names(&effective_tool_view);

    Ok(json!({
        "has_policy": session_tool_policy.is_some(),
        "updated_at": updated_at,
        "requested_tool_ids": requested_tool_ids,
        "visible_requested_tool_ids": visible_tool_id_names(&requested_tool_ids),
        "base_tool_ids": base_tool_ids,
        "visible_base_tool_ids": visible_tool_id_names(&base_tool_ids),
        "effective_tool_ids": effective_tool_ids,
        "visible_effective_tool_ids": visible_tool_id_names(&effective_tool_ids),
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
fn normalize_required_task_id(task_id: &str, field: &str) -> Result<String, String> {
    let trimmed = task_id.trim();
    if trimmed.is_empty() {
        return Err(format!("task tool requires payload.{field}"));
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
fn parse_task_target_request(
    payload: &Value,
    task_field: &str,
    task_list_field: Option<&str>,
) -> Result<TaskTargetRequest, String> {
    let single = optional_payload_string(payload, task_field);
    let batch = match task_list_field {
        Some(task_list_field) => optional_payload_string_array(payload, task_list_field)?,
        None => None,
    };

    match (single, batch) {
        (Some(task_id), None) => Ok(TaskTargetRequest {
            task_ids: vec![normalize_required_task_id(&task_id, task_field)?],
            legacy_single: true,
        }),
        (None, Some(task_ids)) => Ok(TaskTargetRequest {
            task_ids,
            legacy_single: false,
        }),
        (Some(_), Some(_)) => Err(format!(
            "task tool requires exactly one of payload.{task_field} or payload.{}",
            task_list_field.unwrap_or("task_ids")
        )),
        (None, None) => Err(format!(
            "task tool requires payload.{task_field}{}",
            task_list_field
                .map(|field| format!(" or payload.{field}"))
                .unwrap_or_default()
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_tasks_list_request(payload: &Value, tool_config: &ToolConfig) -> TasksListRequest {
    TasksListRequest {
        limit: optional_payload_limit(
            payload,
            "limit",
            tool_config.sessions.history_limit.min(50),
            tool_config.sessions.history_limit,
        ),
        offset: optional_payload_offset(payload, "offset", 0),
        task_state: optional_payload_string(payload, "task_state")
            .map(|value| value.to_ascii_lowercase()),
        stable_only: payload
            .get("stable_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

#[cfg(feature = "memory-sqlite")]
fn parse_tasks_search_request(
    payload: &Value,
    tool_config: &ToolConfig,
) -> Result<TasksSearchRequest, String> {
    let query = required_payload_string(payload, "query", "task tool")?;
    Ok(TasksSearchRequest {
        query,
        max_results: optional_payload_limit(
            payload,
            "max_results",
            tool_config.sessions.history_limit.min(20),
            tool_config.sessions.history_limit.min(50),
        ),
        task_state: optional_payload_string(payload, "task_state")
            .map(|value| value.to_ascii_lowercase()),
        stable_only: payload
            .get("stable_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
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
fn legacy_single_task_id(task_ids: &[String]) -> Result<&str, String> {
    task_ids
        .first()
        .map(String::as_str)
        .ok_or_else(|| "task_tool_internal_error: legacy single request missing task id".to_owned())
}

#[cfg(feature = "memory-sqlite")]
fn legacy_single_task_target(
    task_targets: &[ResolvedTaskTarget],
) -> Result<&ResolvedTaskTarget, String> {
    task_targets.first().ok_or_else(|| {
        "task_tool_internal_error: legacy single request missing resolved task".to_owned()
    })
}

#[cfg(feature = "memory-sqlite")]
fn resolve_task_targets(
    repo: &SessionRepository,
    current_session_id: &str,
    task_ids: &[String],
    tool_config: &ToolConfig,
) -> Result<Vec<ResolvedTaskTarget>, String> {
    let visible_task_records = load_visible_task_records(repo, current_session_id)?;

    task_ids
        .iter()
        .map(|task_id| {
            resolve_task_target_from_visible_records(
                repo,
                current_session_id,
                task_id,
                tool_config,
                &visible_task_records,
            )
        })
        .collect()
}

#[cfg(feature = "memory-sqlite")]
fn load_visible_task_records(
    repo: &SessionRepository,
    current_session_id: &str,
) -> Result<Vec<VisibleTaskRecord>, String> {
    let visible_sessions = repo.list_visible_sessions(current_session_id)?;
    let mut tasks_by_id = BTreeMap::<String, VisibleTaskRecord>::new();

    for session in visible_sessions {
        let workflow = load_session_workflow_record(repo, &session, None)?;
        let Some(task_progress) = workflow.task_progress else {
            continue;
        };

        let task_id = task_progress.task_id.trim().to_owned();
        if task_id.is_empty() {
            continue;
        }

        let candidate = VisibleTaskRecord {
            task_id: task_id.clone(),
            owner_session_id: session.session_id.clone(),
            session_label: session.label.clone(),
            session_updated_at: session.updated_at,
            task_progress,
        };
        let should_replace = tasks_by_id
            .get(task_id.as_str())
            .map(|existing| visible_task_record_is_newer(&candidate, existing))
            .unwrap_or(true);
        if should_replace {
            tasks_by_id.insert(task_id, candidate);
        }
    }

    let mut tasks = tasks_by_id.into_values().collect::<Vec<_>>();
    tasks.sort_by(visible_task_record_cmp_desc);
    Ok(tasks)
}

#[cfg(feature = "memory-sqlite")]
fn visible_task_record_is_newer(
    candidate: &VisibleTaskRecord,
    existing: &VisibleTaskRecord,
) -> bool {
    visible_task_record_cmp_desc(candidate, existing).is_lt()
}

#[cfg(feature = "memory-sqlite")]
fn visible_task_record_cmp_desc(
    left: &VisibleTaskRecord,
    right: &VisibleTaskRecord,
) -> std::cmp::Ordering {
    right
        .task_progress
        .updated_at
        .cmp(&left.task_progress.updated_at)
        .then_with(|| right.session_updated_at.cmp(&left.session_updated_at))
        .then_with(|| left.task_id.cmp(&right.task_id))
        .then_with(|| left.owner_session_id.cmp(&right.owner_session_id))
}

#[cfg(feature = "memory-sqlite")]
fn resolve_task_target(
    repo: &SessionRepository,
    current_session_id: &str,
    task_id: &str,
    tool_config: &ToolConfig,
) -> Result<ResolvedTaskTarget, String> {
    resolve_task_targets(repo, current_session_id, &[task_id.to_owned()], tool_config)?
        .into_iter()
        .next()
        .ok_or_else(|| "task_tool_internal_error: expected a resolved task target".to_owned())
}

#[cfg(feature = "memory-sqlite")]
fn resolve_task_target_from_visible_records(
    repo: &SessionRepository,
    current_session_id: &str,
    requested_task_id: &str,
    tool_config: &ToolConfig,
    visible_task_records: &[VisibleTaskRecord],
) -> Result<ResolvedTaskTarget, String> {
    let resolved_match = visible_task_records
        .iter()
        .find(|visible_task| visible_task.task_id == requested_task_id);
    if let Some(visible_task) = resolved_match {
        return Ok(ResolvedTaskTarget {
            task_id: visible_task.task_id.clone(),
            owner_session_id: visible_task.owner_session_id.clone(),
        });
    }

    let visible_sessions = repo.list_visible_sessions(current_session_id)?;
    let mut resolved_binding_match = None::<(i64, String, String)>;
    for session in visible_sessions {
        let workflow = load_session_workflow_record(repo, &session, None)?;
        let Some(binding) = workflow.binding.as_ref() else {
            continue;
        };
        if binding.task_id != requested_task_id {
            continue;
        }

        let candidate = (
            session.updated_at,
            binding.task_id.clone(),
            session.session_id.clone(),
        );
        let should_replace = resolved_binding_match
            .as_ref()
            .map(|existing| candidate > *existing)
            .unwrap_or(true);
        if should_replace {
            resolved_binding_match = Some(candidate);
        }
    }
    if let Some((_, task_id, owner_session_id)) = resolved_binding_match {
        return Ok(ResolvedTaskTarget {
            task_id,
            owner_session_id,
        });
    }

    let session = repo
        .load_session_summary_with_legacy_fallback(requested_task_id)?
        .ok_or_else(|| format!("task_not_found: `{requested_task_id}`"))?;
    ensure_visible(
        repo,
        current_session_id,
        &session.session_id,
        tool_config.sessions.visibility,
    )?;
    let workflow = load_session_workflow_record(repo, &session, None)?;
    let task_id = workflow
        .task_progress
        .as_ref()
        .map(|task_progress| task_progress.task_id.clone())
        .or_else(|| {
            workflow
                .binding
                .as_ref()
                .map(|binding| binding.task_id.clone())
        })
        .unwrap_or_else(|| requested_task_id.to_owned());

    Ok(ResolvedTaskTarget {
        task_id,
        owner_session_id: session.session_id,
    })
}

#[cfg(feature = "memory-sqlite")]
fn load_task_lineage_records(
    repo: &SessionRepository,
    current_session_id: &str,
    resolved_target: &ResolvedTaskTarget,
) -> Result<Vec<VisibleTaskSessionRecord>, String> {
    let visible_sessions = repo.list_visible_sessions(current_session_id)?;
    let mut lineage_records = Vec::new();

    for session in visible_sessions {
        let task_identity = resolve_task_identity_for_session(repo, &session.session_id);
        if task_identity.task_id != resolved_target.task_id {
            continue;
        }

        let workflow = load_session_workflow_record(repo, &session, None)?;
        let lineage_event_id = latest_task_lineage_event_id(
            repo,
            &session.session_id,
            task_identity.task_id.as_str(),
        )?;
        let lineage_record = VisibleTaskSessionRecord {
            task_id: task_identity.task_id,
            owner_session_id: session.session_id.clone(),
            task_session_id: task_identity.task_session_id,
            session_label: session.label.clone(),
            session_state: session.state,
            archived: session.archived_at.is_some(),
            lineage_event_id,
            session_updated_at: session.updated_at,
            task_progress: workflow.task_progress,
        };
        lineage_records.push(lineage_record);
    }

    if lineage_records.is_empty() {
        let session = repo
            .load_session_summary_with_legacy_fallback(&resolved_target.owner_session_id)?
            .ok_or_else(|| {
                format!(
                    "task_history_internal_error: missing owner session `{}`",
                    resolved_target.owner_session_id
                )
            })?;
        let workflow = load_session_workflow_record(repo, &session, None)?;
        let task_identity = resolve_task_identity_for_session(repo, &session.session_id);
        let lineage_event_id = latest_task_lineage_event_id(
            repo,
            &session.session_id,
            task_identity.task_id.as_str(),
        )?;
        let lineage_record = VisibleTaskSessionRecord {
            task_id: resolved_target.task_id.clone(),
            owner_session_id: session.session_id.clone(),
            task_session_id: task_identity.task_session_id,
            session_label: session.label.clone(),
            session_state: session.state,
            archived: session.archived_at.is_some(),
            lineage_event_id,
            session_updated_at: session.updated_at,
            task_progress: workflow.task_progress,
        };
        lineage_records.push(lineage_record);
    }

    lineage_records.sort_by(task_session_record_cmp_asc);
    Ok(lineage_records)
}

#[cfg(feature = "memory-sqlite")]
fn task_session_record_cmp_asc(
    left: &VisibleTaskSessionRecord,
    right: &VisibleTaskSessionRecord,
) -> std::cmp::Ordering {
    left.lineage_event_id
        .cmp(&right.lineage_event_id)
        .then_with(|| left.task_session_id.cmp(&right.task_session_id))
        .then_with(|| left.owner_session_id.cmp(&right.owner_session_id))
}

#[cfg(feature = "memory-sqlite")]
fn latest_task_lineage_event_id(
    repo: &SessionRepository,
    session_id: &str,
    task_id: &str,
) -> Result<i64, String> {
    let session_events = repo.list_recent_events(session_id, 200)?;
    for session_event in session_events.iter().rev() {
        let task_identity = resolve_task_identity_for_event(
            session_event.event_kind.as_str(),
            &session_event.payload_json,
            session_id,
        );
        let Some(task_identity) = task_identity else {
            continue;
        };
        if task_identity.task_id == task_id {
            return Ok(session_event.id);
        }
    }

    Ok(0)
}

#[cfg(feature = "memory-sqlite")]
fn load_task_history_turns(
    config: &SessionStoreConfig,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let mut turns = Vec::new();

    for (lineage_index, lineage_record) in lineage_records.iter().enumerate() {
        let session_turns =
            store::window_session_turns(&lineage_record.owner_session_id, limit, config)
                .map_err(|error| format!("load task transcript failed: {error}"))?;
        for (turn_index, session_turn) in session_turns.into_iter().enumerate() {
            let turn_payload = json!({
                "task_session_id": lineage_record.task_session_id,
                "owner_session_id": lineage_record.owner_session_id,
                "session_label": lineage_record.session_label,
                "is_current_owner": lineage_record.owner_session_id == current_owner_session_id,
                "role": session_turn.role,
                "content": session_turn.content,
                "ts": session_turn.ts,
                "__lineage_order": lineage_index,
                "__turn_order": turn_index,
            });
            turns.push(turn_payload);
        }
    }

    turns.sort_by(task_turn_json_cmp_asc);
    truncate_sorted_tail(&mut turns, limit);
    for turn in &mut turns {
        let Some(turn_object) = turn.as_object_mut() else {
            continue;
        };
        turn_object.remove("__lineage_order");
        turn_object.remove("__turn_order");
    }
    Ok(turns)
}

#[cfg(feature = "memory-sqlite")]
fn task_turn_json_cmp_asc(left: &Value, right: &Value) -> std::cmp::Ordering {
    let left_ts = left.get("ts").and_then(Value::as_i64).unwrap_or(i64::MIN);
    let right_ts = right.get("ts").and_then(Value::as_i64).unwrap_or(i64::MIN);
    let left_lineage_order = left
        .get("__lineage_order")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let right_lineage_order = right
        .get("__lineage_order")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let left_turn_order = left
        .get("__turn_order")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let right_turn_order = right
        .get("__turn_order")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);

    left_ts
        .cmp(&right_ts)
        .then_with(|| left_lineage_order.cmp(&right_lineage_order))
        .then_with(|| left_turn_order.cmp(&right_turn_order))
        .then_with(|| {
            let left_session = left
                .get("task_session_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let right_session = right
                .get("task_session_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_session.cmp(right_session)
        })
        .then_with(|| {
            let left_role = left.get("role").and_then(Value::as_str).unwrap_or_default();
            let right_role = right
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            left_role.cmp(right_role)
        })
}

#[cfg(feature = "memory-sqlite")]
fn load_task_history_events(
    repo: &SessionRepository,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    after_id: Option<i64>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let mut task_events = Vec::new();

    for lineage_record in lineage_records {
        let session_events = match after_id {
            Some(after_id) => {
                repo.list_events_after(&lineage_record.owner_session_id, after_id.max(0), limit)?
            }
            None => repo.list_recent_events(&lineage_record.owner_session_id, limit)?,
        };
        for session_event in session_events {
            let task_identity = resolve_task_identity_for_event(
                session_event.event_kind.as_str(),
                &session_event.payload_json,
                &lineage_record.owner_session_id,
            );
            let Some(task_identity) = task_identity else {
                continue;
            };
            if task_identity.task_id != lineage_record.task_id {
                continue;
            }

            let mut event_payload = session_event_json(session_event);
            if let Some(event_object) = event_payload.as_object_mut() {
                event_object.insert(
                    "task_session_id".to_owned(),
                    Value::String(task_identity.task_session_id),
                );
                event_object.insert(
                    "session_label".to_owned(),
                    lineage_record
                        .session_label
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
                event_object.insert(
                    "is_current_owner".to_owned(),
                    Value::Bool(lineage_record.owner_session_id == current_owner_session_id),
                );
            }
            task_events.push(event_payload);
        }
    }

    task_events.sort_by(task_event_json_cmp_asc);
    truncate_sorted_tail(&mut task_events, limit);
    Ok(task_events)
}

#[cfg(feature = "memory-sqlite")]
fn load_task_event_window(
    repo: &SessionRepository,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    after_id: Option<i64>,
    limit: usize,
) -> Result<(Vec<Value>, i64), String> {
    let events = load_task_history_events(
        repo,
        lineage_records,
        current_owner_session_id,
        after_id,
        limit,
    )?;
    let next_after_id = events
        .last()
        .and_then(|event| event.get("id"))
        .and_then(Value::as_i64)
        .unwrap_or(after_id.unwrap_or(0));

    Ok((events, next_after_id))
}

#[cfg(feature = "memory-sqlite")]
fn task_event_json_cmp_asc(left: &Value, right: &Value) -> std::cmp::Ordering {
    let left_ts = left.get("ts").and_then(Value::as_i64).unwrap_or(i64::MIN);
    let right_ts = right.get("ts").and_then(Value::as_i64).unwrap_or(i64::MIN);
    let left_id = left.get("id").and_then(Value::as_i64).unwrap_or(i64::MIN);
    let right_id = right.get("id").and_then(Value::as_i64).unwrap_or(i64::MIN);

    left_ts.cmp(&right_ts).then_with(|| left_id.cmp(&right_id))
}

#[cfg(feature = "memory-sqlite")]
fn truncate_sorted_tail(items: &mut Vec<Value>, limit: usize) {
    if items.len() <= limit {
        return;
    }

    let keep_from = items.len().saturating_sub(limit);
    let retained_items = items.split_off(keep_from);
    *items = retained_items;
}

#[cfg(feature = "memory-sqlite")]
fn task_session_summary_json(
    lineage_record: &VisibleTaskSessionRecord,
    current_owner_session_id: &str,
) -> Value {
    let task_state = lineage_record
        .task_progress
        .as_ref()
        .map(|task_progress| task_progress.status.as_str().to_owned());
    let verification_state = lineage_record
        .task_progress
        .as_ref()
        .and_then(|task_progress| task_progress.verification_state)
        .map(|value| value.as_str().to_owned());

    json!({
        "task_id": lineage_record.task_id,
        "task_session_id": lineage_record.task_session_id,
        "owner_session_id": lineage_record.owner_session_id,
        "session_label": lineage_record.session_label,
        "session_state": lineage_record.session_state.as_str(),
        "archived": lineage_record.archived,
        "is_current_owner": lineage_record.owner_session_id == current_owner_session_id,
        "updated_at": lineage_record
            .task_progress
            .as_ref()
            .map(|task_progress| task_progress.updated_at)
            .unwrap_or(lineage_record.session_updated_at),
        "task_state": task_state,
        "verification_state": verification_state,
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
        offset: optional_payload_offset(payload, "offset", 0),
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
        if wait_status != "completed" {
            let continuation_note =
                continuation_note_for_wait_status(wait_status, object.get("session"));
            object.insert(
                "continuation".to_owned(),
                json!({
                    "state": wait_status,
                    "is_terminal": false,
                    "note": continuation_note,
                }),
            );
        }
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
fn continuation_note_for_wait_status(wait_status: &str, session_payload: Option<&Value>) -> String {
    let session_state = session_payload
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match wait_status {
        "waiting" => format!(
            "The runtime is still waiting on session state `{session_state}`. Treat this as intermediate progress, not final completion."
        ),
        "blocked" => format!(
            "The runtime is blocked while the session state is `{session_state}`. Report the exact blocker or resolve it before presenting final completion."
        ),
        other => format!(
            "The runtime is still in non-terminal wait state `{other}` with session state `{session_state}`."
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
fn task_wait_outcome(
    status: &str,
    snapshot: SessionInspectionSnapshot,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: status.to_owned(),
        payload: task_wait_payload(
            snapshot,
            lineage_records,
            current_owner_session_id,
            wait_status,
            after_id,
            timeout_ms,
            observed_events,
            next_after_id,
        ),
    }
}

#[cfg(feature = "memory-sqlite")]
fn task_wait_payload(
    snapshot: SessionInspectionSnapshot,
    lineage_records: &[VisibleTaskSessionRecord],
    current_owner_session_id: &str,
    wait_status: &str,
    after_id: Option<i64>,
    timeout_ms: u64,
    observed_events: Vec<SessionEventRecord>,
    next_after_id: i64,
) -> Value {
    let payload = wait_payload(
        snapshot,
        wait_status,
        after_id,
        timeout_ms,
        observed_events,
        next_after_id,
    );
    let task_events = payload
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    event.get("event_kind").and_then(Value::as_str)
                        == Some(TASK_PROGRESS_EVENT_KIND)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut payload = rewrite_task_payload_aliases(payload, "task_wait");
    let task_state = task_state_from_payload(&payload);
    if let Some(object) = payload.as_object_mut() {
        object.insert("task_events".to_owned(), Value::Array(task_events));
    }
    let payload = decorate_task_status_payload(payload, task_state);
    decorate_task_lineage_payload(payload, lineage_records, current_owner_session_id)
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
        "offset": request.offset,
        "state": request.state.map(SessionState::as_str),
        "kind": request.kind.map(SessionKind::as_str),
        "parent_session_id": request.parent_session_id.clone(),
        "overdue_only": request.overdue_only,
        "include_archived": request.include_archived,
        "include_delegate_lifecycle": request.effective_include_delegate_lifecycle(),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json(session: SessionSummaryRecord, workflow: SessionWorkflowRecord) -> Value {
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
        "workflow": session_workflow_json(workflow),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_summary_json_with_delegate_lifecycle(
    session: SessionSummaryRecord,
    workflow: SessionWorkflowRecord,
    delegate_lifecycle: Option<SessionDelegateLifecycleRecord>,
    subagent_contract: Option<ConstrainedSubagentContractView>,
    include_delegate_lifecycle: bool,
) -> Value {
    let subagent = subagent_handle_for_session(
        &session,
        subagent_contract.as_ref(),
        delegate_lifecycle.as_ref(),
    );
    let mut payload = session_summary_json(session, workflow);
    if let Some(object) = payload.as_object_mut() {
        insert_subagent_surface_fields(object, subagent_contract.as_ref(), subagent.as_ref());
        if include_delegate_lifecycle {
            object.insert(
                "delegate_lifecycle".to_owned(),
                delegate_lifecycle
                    .map(|lifecycle| {
                        session_delegate_lifecycle_json(lifecycle, subagent_contract.as_ref())
                    })
                    .unwrap_or(Value::Null),
            );
        }
    }
    payload
}

#[cfg(feature = "memory-sqlite")]
fn insert_subagent_surface_fields(
    object: &mut serde_json::Map<String, Value>,
    _subagent_contract: Option<&ConstrainedSubagentContractView>,
    subagent: Option<&ConstrainedSubagentHandle>,
) {
    object.extend(subagent_surface_fields(subagent));
}

#[cfg(feature = "memory-sqlite")]
fn subagent_handle_for_session(
    session: &SessionSummaryRecord,
    subagent_contract: Option<&ConstrainedSubagentContractView>,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
) -> Option<ConstrainedSubagentHandle> {
    if session.kind != SessionKind::DelegateChild {
        return None;
    }

    let phase = delegate_lifecycle.map(|lifecycle| lifecycle.phase.to_owned());
    Some(
        ConstrainedSubagentHandle::new(session.session_id.clone())
            .with_parent_session_id(session.parent_session_id.clone())
            .with_label(session.label.clone())
            .with_state(Some(session.state.as_str().to_owned()))
            .with_phase(phase.clone())
            .with_identity(
                subagent_contract
                    .and_then(ConstrainedSubagentContractView::resolved_identity)
                    .cloned(),
            )
            .with_contract(subagent_contract.cloned())
            .with_coordination(subagent_handle_coordination_actions(
                session,
                phase.as_deref(),
                delegate_lifecycle,
                subagent_contract,
            )),
    )
}

#[cfg(feature = "memory-sqlite")]
fn subagent_handle_coordination_actions(
    session: &SessionSummaryRecord,
    phase: Option<&str>,
    delegate_lifecycle: Option<&SessionDelegateLifecycleRecord>,
    subagent_contract: Option<&ConstrainedSubagentContractView>,
) -> Vec<crate::conversation::ConstrainedSubagentCoordinationAction> {
    let is_async = delegate_lifecycle
        .map(|lifecycle| lifecycle.mode == "async")
        .unwrap_or_else(|| {
            matches!(
                subagent_contract.and_then(|contract| contract.mode),
                Some(crate::conversation::ConstrainedSubagentMode::Async)
            )
        });
    let overdue = matches!(
        delegate_lifecycle.and_then(|lifecycle| lifecycle.staleness.as_ref()),
        Some(staleness) if staleness.state == "overdue"
    );
    let mode = if is_async {
        Some(crate::conversation::ConstrainedSubagentMode::Async)
    } else {
        subagent_contract.and_then(|contract| contract.mode)
    };
    let terminal_but_not_archived =
        session_state_is_terminal(session.state) && session.archived_at.is_none();
    coordination_actions_for_subagent_handle(terminal_but_not_archived, phase, mode, overdue)
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_json(workflow: SessionWorkflowRecord) -> Value {
    json!({
        "workflow_id": workflow.workflow_id,
        "task": workflow.task,
        "phase": workflow.phase,
        "operation_kind": workflow.operation_kind,
        "operation_scope": workflow.operation_scope,
        "task_session_id": workflow.task_session_id,
        "lineage_root_session_id": workflow.lineage_root_session_id,
        "lineage_depth": workflow.lineage_depth,
        "task_progress": workflow.task_progress,
        "runtime_self_continuity": workflow
            .runtime_self_continuity
            .map(session_runtime_self_continuity_json),
        "binding": workflow.binding.map(session_workflow_binding_json),
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_runtime_self_continuity_json(
    runtime_self_continuity: SessionRuntimeSelfContinuityRecord,
) -> Value {
    json!({
        "present": runtime_self_continuity.present,
        "resolved_identity_present": runtime_self_continuity.resolved_identity_present,
        "session_profile_projection_present": runtime_self_continuity
            .session_profile_projection_present,
    })
}

#[cfg(feature = "memory-sqlite")]
fn session_workflow_binding_json(binding: SessionWorkflowBindingRecord) -> Value {
    json!({
        "session_id": binding.session_id,
        "task_id": binding.task_id,
        "task_session_id": binding.task_session_id,
        "mode": binding.mode,
        "execution_surface": binding.execution_surface,
        "worktree": binding.worktree,
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
        "frozen_result": outcome.frozen_result,
        "recorded_at": outcome.recorded_at,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
    use loong_kernel::mailbox::{AgentPath, MailboxContent};
    use rusqlite::params;
    use serde_json::{Value, json};
    use tokio::time::{Duration, Instant, sleep};

    use crate::config::{SessionVisibility, ToolConfig};
    use crate::conversation::{InterAgentMessage, mailbox_for_session};
    use crate::session::repository::{
        FinalizeSessionTerminalRequest, NewSessionEvent, NewSessionRecord, SessionEventRecord,
        SessionKind, SessionRepository, SessionState, SessionSummaryRecord,
    };
    use crate::session::store::{SessionStoreConfig, append_session_turn_direct};

    use super::{
        execute_session_tool_with_config, execute_session_tool_with_policies,
        wait_for_single_session_with_policies,
    };

    fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
        let base = std::env::temp_dir().join(format!(
            "loong-session-tools-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        SessionStoreConfig {
            sqlite_path: Some(db_path),
            runtime_config: None,
        }
    }

    fn execute_session_mutation_tool_with_config(
        request: ToolCoreRequest,
        current_session_id: &str,
        config: &SessionStoreConfig,
    ) -> Result<ToolCoreOutcome, String> {
        let mut tool_config = ToolConfig::default();
        tool_config.sessions.allow_mutation = true;
        execute_session_tool_with_policies(request, current_session_id, config, &tool_config)
    }

    fn overwrite_session_event_ts(
        config: &SessionStoreConfig,
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

    fn overwrite_session_updated_at(config: &SessionStoreConfig, session_id: &str, ts: i64) {
        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path for session tools test");
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET updated_at = ?2
                 WHERE session_id = ?1",
                params![session_id, ts],
            )
            .expect("update session updated_at");
        assert!(updated > 0, "expected at least one updated session row");
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
    fn session_mutation_tools_can_be_explicitly_disabled() {
        let config = isolated_memory_config("session-mutation-disabled");
        let mut tool_config = ToolConfig::default();
        tool_config.sessions.allow_mutation = false;
        for tool_name in ["session_archive", "session_cancel", "session_recover"] {
            let error = execute_session_tool_with_policies(
                ToolCoreRequest {
                    tool_name: tool_name.to_owned(),
                    payload: json!({
                        "session_id": "child-session"
                    }),
                },
                "root-session",
                &config,
                &tool_config,
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

        append_session_turn_direct("root-session", "user", "root turn", &config)
            .expect("append root turn");
        append_session_turn_direct("child-session", "assistant", "child turn", &config)
            .expect("append child turn");
        append_session_turn_direct("other-session", "user", "other turn", &config)
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
                    frozen_result: None,
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
                frozen_result: None,
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
        let coordination = archived["subagent"]["coordination"]
            .as_array()
            .expect("coordination actions");
        let archive_actions = coordination
            .iter()
            .filter(|action| action["tool_name"] == "session_archive")
            .count();
        assert_eq!(outcome.payload["filters"]["include_archived"], true);
        assert_eq!(archived["archived"], true);
        assert!(archived["archived_at"].is_number());
        assert_eq!(archived["subagent"]["session_id"], "archived-child");
        assert_eq!(archive_actions, 0);
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
    fn sessions_list_applies_offset_pagination() {
        let config = isolated_memory_config("sessions-list-offset");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "000-root".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        for session_id in ["001-child", "002-child", "003-child"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("000-root".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }

        overwrite_session_updated_at(&config, "000-root", 400);
        overwrite_session_updated_at(&config, "001-child", 300);
        overwrite_session_updated_at(&config, "002-child", 200);
        overwrite_session_updated_at(&config, "003-child", 100);

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({
                    "limit": 2,
                    "offset": 1
                }),
            },
            "000-root",
            &config,
        )
        .expect("sessions_list outcome");

        let sessions_value = &outcome.payload["sessions"];
        let sessions = sessions_value.as_array().expect("sessions array");
        let mut ids = Vec::new();
        for item in sessions {
            let session_id_value = item.get("session_id");
            let Some(session_id_value) = session_id_value else {
                continue;
            };
            let session_id = session_id_value.as_str();
            let Some(session_id) = session_id else {
                continue;
            };
            ids.push(session_id);
        }

        let filter_offset_value = &outcome.payload["filters"]["offset"];
        let matched_count_value = &outcome.payload["matched_count"];
        let returned_count_value = &outcome.payload["returned_count"];
        let has_more_value = &outcome.payload["has_more"];
        let filter_offset = filter_offset_value.as_u64().expect("filter offset");
        let matched_count = matched_count_value.as_u64().expect("matched count");
        let returned_count = returned_count_value.as_u64().expect("returned count");
        let has_more = has_more_value.as_bool().expect("has more");

        assert_eq!(ids, vec!["001-child", "002-child"]);
        assert_eq!(filter_offset, 1);
        assert_eq!(matched_count, 4);
        assert_eq!(returned_count, 2);
        assert!(has_more);
    }

    #[test]
    fn sessions_list_includes_workflow_metadata_for_delegate_children() {
        let config = isolated_memory_config("sessions-list-workflow");
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
            label: Some("Research Child".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research release readiness",
                "task_scope": {
                    "task_id": "task-release-readiness"
                },
                "task_session_id": "child-session",
                "label": "Research Child",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 120,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "workspace_root": "/tmp/loong/sessions-list-workflow/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                }
            }),
        })
        .expect("append queued");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({
                    "kind": "delegate_child"
                }),
            },
            "root-session",
            &config,
        )
        .expect("sessions_list outcome");

        let child = outcome.payload["sessions"]
            .as_array()
            .expect("sessions array")
            .iter()
            .find(|item| item["session_id"] == "child-session")
            .expect("child session");
        assert_eq!(child["workflow"]["workflow_id"], "root-session");
        assert_eq!(child["workflow"]["task"], "research release readiness");
        assert_eq!(child["workflow"]["phase"], "execute");
        assert_eq!(child["workflow"]["operation_kind"], "task");
        assert_eq!(child["workflow"]["operation_scope"], "task");
        assert_eq!(child["workflow"]["task_session_id"], "child-session");
        assert_eq!(child["workflow"]["lineage_root_session_id"], "root-session");
        assert_eq!(child["workflow"]["lineage_depth"], 1);
        assert_eq!(child["workflow"]["binding"]["session_id"], "child-session");
        assert_eq!(
            child["workflow"]["binding"]["task_id"],
            "task-release-readiness"
        );
        assert_eq!(
            child["workflow"]["binding"]["task_session_id"],
            "child-session"
        );
        assert_eq!(child["workflow"]["binding"]["mode"], "advisory_only");
        assert_eq!(
            child["workflow"]["binding"]["execution_surface"],
            "delegate.async"
        );
        assert_eq!(
            child["workflow"]["binding"]["worktree"]["worktree_id"],
            "child-session"
        );
        assert_eq!(child["subagent"]["session_id"], "child-session");
        assert_eq!(child["subagent_identity"]["nickname"], "Research Child");
        assert_eq!(
            child["subagent_contract"]["profile"]["role"],
            "orchestrator"
        );
        assert_eq!(
            child["subagent_contract"]["profile"]["control_scope"],
            "children"
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

        append_session_turn_direct("child-session", "user", "hello", &config)
            .expect("append user turn");
        append_session_turn_direct("child-session", "assistant", "world", &config)
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
    fn session_status_includes_workflow_metadata_for_delegate_child() {
        let config = isolated_memory_config("session-status-workflow");
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
            label: Some("Continuity Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research continuity",
                "task_scope": {
                    "task_id": "task-continuity"
                },
                "task_session_id": "child-session",
                "label": "Continuity Child",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "workspace_root": "/tmp/loong/session-status-workflow/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                },
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Child"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Child"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append delegate_started");
        append_session_turn_direct("child-session", "user", "hello", &config)
            .expect("append user turn");
        append_session_turn_direct("child-session", "assistant", "world", &config)
            .expect("append assistant turn");

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

        assert_eq!(outcome.payload["workflow"]["workflow_id"], "root-session");
        assert_eq!(outcome.payload["workflow"]["task"], "research continuity");
        assert_eq!(outcome.payload["workflow"]["phase"], "execute");
        assert_eq!(outcome.payload["workflow"]["operation_kind"], "task");
        assert_eq!(outcome.payload["workflow"]["operation_scope"], "task");
        assert_eq!(
            outcome.payload["workflow"]["task_session_id"],
            "child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["lineage_root_session_id"],
            "root-session"
        );
        assert_eq!(outcome.payload["workflow"]["lineage_depth"], 1);
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["session_id"],
            "child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["task_id"],
            "task-continuity"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["task_session_id"],
            "child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["mode"],
            "advisory_only"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["execution_surface"],
            "delegate.async"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["worktree"]["worktree_id"],
            "child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["worktree"]["workspace_root"],
            "/tmp/loong/session-status-workflow/child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
            true
        );
        assert_eq!(outcome.payload["subagent"]["session_id"], "child-session");
        assert_eq!(
            outcome.payload["subagent_identity"]["nickname"],
            "Continuity Child"
        );
        assert_eq!(
            outcome.payload["subagent_contract"]["profile"]["role"],
            "orchestrator"
        );
        assert_eq!(
            outcome.payload["subagent_contract"]["profile"]["control_scope"],
            "children"
        );
        assert_eq!(outcome.payload["session"]["turn_count"], 2);
        assert!(outcome.payload["session"]["last_turn_at"].is_number());
    }

    #[test]
    fn session_status_includes_runtime_self_continuity_from_refresh_events() {
        let config = isolated_memory_config("session-status-refresh-continuity");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
                .to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "source": "compaction",
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Root"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Root"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append runtime self continuity refresh");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
            true
        );
    }

    #[test]
    fn session_status_includes_task_progress_from_latest_event() {
        let config = isolated_memory_config("session-status-task-progress");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "root-session".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Watch long-running task progress".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: vec![crate::task_progress::TaskActiveHandleRecord {
                        handle_kind: "conversation_turn".to_owned(),
                        handle_id: "root-session".to_owned(),
                        state: "waiting".to_owned(),
                        last_event_at: Some(123),
                        stop_condition: "terminal_reply".to_owned(),
                    }],
                    resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                        recommended_tool: "session_wait".to_owned(),
                        session_id: "root-session".to_owned(),
                        note: Some("Wait for durable task-progress transitions.".to_owned()),
                    }),
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["task_id"],
            "root-session"
        );
        assert_eq!(outcome.payload["task_progress"]["task_id"], "root-session");
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["status"],
            "waiting"
        );
        assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["intent_summary"],
            "Watch long-running task progress"
        );
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["verification_state"],
            "pending"
        );
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["active_handles"][0]["handle_kind"],
            "conversation_turn"
        );
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["resume_recipe"]["recommended_tool"],
            "session_wait"
        );
        assert_eq!(
            outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
            "session_wait"
        );
    }

    #[test]
    fn session_status_keeps_runtime_self_continuity_after_more_than_64_newer_events() {
        let config = isolated_memory_config("session-status-refresh-continuity-stale-window");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
                .to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "source": "compaction",
                "runtime_self_continuity": {
                    "runtime_self": {
                        "standing_instructions": ["Stay concise."],
                        "tool_usage_policy": ["Prefer visible evidence."],
                        "soul_guidance": ["Keep continuity explicit."],
                        "identity_context": ["# Identity\n- Name: Root"],
                        "user_context": ["Operator prefers concise technical summaries."]
                    },
                    "resolved_identity": {
                        "source": "workspace_self",
                        "content": "# Identity\n- Name: Root"
                    },
                    "session_profile_projection": "## Session Profile\nOperator prefers concise technical summaries."
                }
            }),
        })
        .expect("append runtime self continuity refresh");

        for index in 0..70 {
            let event_kind = format!("noise_event_{index}");
            let payload = json!({ "index": index });
            let event = NewSessionEvent {
                session_id: "root-session".to_owned(),
                event_kind,
                actor_session_id: Some("root-session".to_owned()),
                payload_json: payload,
            };
            repo.append_event(event).expect("append noise event");
        }

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["resolved_identity_present"],
            true
        );
        assert_eq!(
            outcome.payload["workflow"]["runtime_self_continuity"]["session_profile_projection_present"],
            true
        );
    }

    #[test]
    fn session_status_keeps_task_progress_outside_recent_event_window() {
        let config = isolated_memory_config("session-status-task-progress-stale-window");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "root-session".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Keep durable task progress visible".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                        recommended_tool: "session_status".to_owned(),
                        session_id: "root-session".to_owned(),
                        note: Some(
                            "Use session_status even after the recent window moves on.".to_owned(),
                        ),
                    }),
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");

        for index in 0..80 {
            repo.append_event(NewSessionEvent {
                session_id: "root-session".to_owned(),
                event_kind: format!("noise_event_{index}"),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({ "index": index }),
            })
            .expect("append noise event");
        }

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["status"],
            "waiting"
        );
        assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["intent_summary"],
            "Keep durable task progress visible"
        );
        assert_eq!(
            outcome.payload["workflow"]["task_progress"]["resume_recipe"]["recommended_tool"],
            "session_status"
        );
        assert_eq!(
            outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
            "session_status"
        );
    }

    #[test]
    fn task_status_resolves_canonical_task_id_and_exposes_owner_session_id() {
        let config = isolated_memory_config("task-status-aliases");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "task-owner".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Task Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "task-owner".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-owner".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Task tool status".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                        recommended_tool: "task_wait".to_owned(),
                        session_id: "task-owner".to_owned(),
                        note: Some("Wait on the task surface.".to_owned()),
                    }),
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_status".to_owned(),
                payload: json!({
                    "task_id": "task-root"
                }),
            },
            "task-owner",
            &config,
        )
        .expect("task_status outcome");

        assert_eq!(outcome.payload["tool"], "task_status");
        assert_eq!(outcome.payload["task_id"], "task-root");
        assert_eq!(outcome.payload["owner_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["session"]["session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_count"], 1);
        assert_eq!(
            outcome.payload["task_sessions"][0]["task_session_id"],
            "task-owner"
        );
        assert_eq!(
            outcome.payload["task_sessions"][0]["is_current_owner"],
            true
        );
        assert_eq!(outcome.payload["task_state"], "waiting");
        assert_eq!(outcome.payload["task_is_stable"], true);
        assert_eq!(outcome.payload["task_progress"]["status"], "waiting");
        assert_eq!(
            outcome.payload["task_progress"]["resume_recipe"]["recommended_tool"],
            "task_wait"
        );
    }

    #[test]
    fn task_status_resolves_binding_only_task_identity_before_task_progress_exists() {
        let config = isolated_memory_config("task-status-binding-only");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
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
                "task": "binding only task",
                "task_scope": {
                    "task_id": "task-bind-only"
                },
                "task_session_id": "child-session",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "workspace_root": "/tmp/loong/task-status-binding-only/child-session",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                }
            }),
        })
        .expect("append delegate_queued");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_status".to_owned(),
                payload: json!({
                    "task_id": "task-bind-only"
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_status outcome");

        assert_eq!(outcome.payload["tool"], "task_status");
        assert_eq!(outcome.payload["task_id"], "task-bind-only");
        assert_eq!(outcome.payload["owner_session_id"], "child-session");
        assert_eq!(outcome.payload["task_session_id"], "child-session");
        assert_eq!(outcome.payload["task_session_count"], 1);
        assert_eq!(
            outcome.payload["task_sessions"][0]["task_session_id"],
            "child-session"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["task_id"],
            "task-bind-only"
        );
        assert_eq!(
            outcome.payload["workflow"]["binding"]["task_session_id"],
            "child-session"
        );
        assert_eq!(outcome.payload["task_state"], "ready");
        assert!(outcome.payload["task_progress"].is_null());
    }

    #[test]
    fn task_history_reads_history_by_canonical_task_id() {
        let config = isolated_memory_config("task-history");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "task-owner".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Task Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "task-owner".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-owner".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Active,
                    intent_summary: Some("Task history".to_owned()),
                    verification_state: Some(
                        crate::task_progress::TaskVerificationState::NotStarted,
                    ),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");
        append_session_turn_direct("task-owner", "user", "hello", &config)
            .expect("append user turn");
        append_session_turn_direct("task-owner", "assistant", "world", &config)
            .expect("append assistant turn");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_history".to_owned(),
                payload: json!({
                    "task_id": "task-root",
                    "limit": 10
                }),
            },
            "task-owner",
            &config,
        )
        .expect("task_history outcome");

        assert_eq!(outcome.payload["tool"], "task_history");
        assert_eq!(outcome.payload["task_id"], "task-root");
        assert_eq!(outcome.payload["owner_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["lineage_session_count"], 1);
        assert_eq!(
            outcome.payload["task_sessions"][0]["task_session_id"],
            "task-owner"
        );
        assert_eq!(
            outcome.payload["task_sessions"][0]["session_state"],
            "running"
        );
        assert_eq!(
            outcome.payload["task_sessions"][0]["is_current_owner"],
            true
        );
        assert_eq!(outcome.payload["turns"][0]["content"], "hello");
        assert_eq!(outcome.payload["turns"][1]["content"], "world");
        assert_eq!(outcome.payload["turns"][0]["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["turns"][1]["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["turns"][0]["is_current_owner"], true);
        assert_eq!(
            outcome.payload["task_events"][0]["event_kind"],
            crate::task_progress::TASK_PROGRESS_EVENT_KIND
        );
        assert_eq!(
            outcome.payload["task_events"][0]["task_session_id"],
            "task-owner"
        );
        assert_eq!(outcome.payload["task_events"][0]["is_current_owner"], true);
    }

    #[test]
    fn task_history_aggregates_visible_task_lineage_across_owner_sessions() {
        let config = isolated_memory_config("task-history-lineage");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for session_id in ["owner-old", "owner-new"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }
        repo.append_event(NewSessionEvent {
            session_id: "owner-old".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-old".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Active,
                    intent_summary: Some("Old owner".to_owned()),
                    verification_state: Some(
                        crate::task_progress::TaskVerificationState::NotStarted,
                    ),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 10,
                },
            ),
        })
        .expect("append old task progress");
        repo.append_event(NewSessionEvent {
            session_id: "owner-new".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "task lineage handoff",
                "task_scope": {
                    "task_id": "task-root"
                },
                "task_session_id": "owner-new",
                "execution": {
                    "mode": "async",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 90,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "workspace_root": "/tmp/loong/task-history-lineage/owner-new",
                    "kernel_bound": false,
                    "runtime_narrowing": {}
                }
            }),
        })
        .expect("append delegate queued");
        repo.append_event(NewSessionEvent {
            session_id: "owner-new".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-new".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Completed,
                    intent_summary: Some("New owner".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Passed),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 20,
                },
            ),
        })
        .expect("append new task progress");
        append_session_turn_direct("owner-old", "user", "old owner turn", &config)
            .expect("append old owner turn");
        append_session_turn_direct("owner-new", "assistant", "new owner turn", &config)
            .expect("append new owner turn");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_history".to_owned(),
                payload: json!({
                    "task_id": "task-root",
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_history outcome");

        assert_eq!(outcome.payload["tool"], "task_history");
        assert_eq!(outcome.payload["task_id"], "task-root");
        assert_eq!(outcome.payload["owner_session_id"], "owner-new");
        assert_eq!(outcome.payload["task_session_id"], "owner-new");
        assert_eq!(outcome.payload["lineage_session_count"], 2);
        let task_sessions = outcome.payload["task_sessions"]
            .as_array()
            .expect("task sessions");
        assert_eq!(task_sessions.len(), 2);
        assert_eq!(task_sessions[0]["task_session_id"], "owner-old");
        assert_eq!(task_sessions[1]["task_session_id"], "owner-new");
        assert_eq!(task_sessions[0]["is_current_owner"], false);
        assert_eq!(task_sessions[1]["is_current_owner"], true);

        let turns = outcome.payload["turns"].as_array().expect("turns");
        let task_turn_sessions = turns
            .iter()
            .map(|turn| {
                turn.get("task_session_id")
                    .and_then(Value::as_str)
                    .expect("task_session_id")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert!(task_turn_sessions.contains(&"owner-old".to_owned()));
        assert!(task_turn_sessions.contains(&"owner-new".to_owned()));

        let task_events = outcome.payload["task_events"]
            .as_array()
            .expect("task events");
        let event_kinds = task_events
            .iter()
            .map(|event| {
                event
                    .get("event_kind")
                    .and_then(Value::as_str)
                    .expect("event kind")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert!(event_kinds.contains(&"delegate_queued".to_owned()));
        assert!(event_kinds.contains(&crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned()));
    }

    #[test]
    fn task_events_supports_lineage_aggregation_and_cursor_follow_up() {
        let config = isolated_memory_config("task-events-lineage");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for session_id in ["owner-old", "owner-new"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }
        repo.append_event(NewSessionEvent {
            session_id: "owner-old".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "task events handoff",
                "task_scope": {
                    "task_id": "task-root"
                },
                "task_session_id": "owner-old"
            }),
        })
        .expect("append delegate queued");
        repo.append_event(NewSessionEvent {
            session_id: "owner-new".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-new".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Completed,
                    intent_summary: Some("Completed by new owner".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Passed),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 20,
                },
            ),
        })
        .expect("append completed task progress");

        let first = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_events".to_owned(),
                payload: json!({
                    "task_id": "task-root",
                    "after_id": 0,
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_events outcome");

        assert_eq!(first.payload["tool"], "task_events");
        assert_eq!(first.payload["task_id"], "task-root");
        assert_eq!(first.payload["owner_session_id"], "owner-new");
        assert_eq!(first.payload["task_session_id"], "owner-new");
        assert_eq!(first.payload["task_session_count"], 2);
        let task_sessions = first.payload["task_sessions"]
            .as_array()
            .expect("task sessions");
        assert_eq!(task_sessions.len(), 2);
        let task_session_ids = task_sessions
            .iter()
            .map(|task_session| {
                task_session
                    .get("task_session_id")
                    .and_then(Value::as_str)
                    .expect("task_session_id")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert!(task_session_ids.contains(&"owner-old".to_owned()));
        assert!(task_session_ids.contains(&"owner-new".to_owned()));
        let current_owner_records = task_sessions
            .iter()
            .filter(|task_session| {
                task_session
                    .get("is_current_owner")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(current_owner_records, 1);
        let events = first.payload["events"].as_array().expect("events");
        assert_eq!(events.len(), 2);
        let next_after_id = first.payload["next_after_id"]
            .as_i64()
            .expect("next_after_id");
        assert!(next_after_id > 0);

        let second = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_events".to_owned(),
                payload: json!({
                    "task_id": "task-root",
                    "after_id": next_after_id,
                    "limit": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_events follow-up outcome");

        assert_eq!(second.payload["events"], json!([]));
        assert_eq!(second.payload["next_after_id"], next_after_id);
        assert_eq!(second.payload["task_session_count"], 2);
    }

    #[test]
    fn task_status_batch_reports_task_ids_without_session_id_aliases() {
        let config = isolated_memory_config("task-status-batch");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");

        for (session_id, task_id, updated_at) in
            [("owner-a", "task-a", 10), ("owner-b", "task-b", 20)]
        {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: task_id.to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Waiting,
                        intent_summary: Some(format!("Status for {task_id}")),
                        verification_state: Some(
                            crate::task_progress::TaskVerificationState::Pending,
                        ),
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at,
                    },
                ),
            })
            .expect("append task progress event");
        }

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_status".to_owned(),
                payload: json!({
                    "task_ids": ["task-a", "task-b"]
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_status batch outcome");

        let results = outcome.payload["results"]
            .as_array()
            .expect("batch results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["task_id"], "task-a");
        assert_eq!(results[0]["owner_session_id"], "owner-a");
        assert_eq!(results[0]["task_session_id"], "owner-a");
        assert_eq!(results[0]["task_session_count"], 1);
        assert_eq!(results[0]["task_sessions"][0]["task_session_id"], "owner-a");
        assert_eq!(results[0]["task_state"], "waiting");
        assert_eq!(results[0]["task_is_stable"], true);
        assert!(results[0].get("session_id").is_none());
        assert_eq!(results[1]["task_id"], "task-b");
        assert_eq!(results[1]["owner_session_id"], "owner-b");
        assert_eq!(results[1]["task_session_id"], "owner-b");
        assert_eq!(results[1]["task_session_count"], 1);
        assert_eq!(results[1]["task_sessions"][0]["task_session_id"], "owner-b");
        assert!(results[1].get("session_id").is_none());
    }

    #[test]
    fn tasks_list_returns_visible_task_progress_records() {
        let config = isolated_memory_config("tasks-list");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for session_id in ["task-a", "task-b", "no-task"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }
        for (session_id, status) in [
            ("task-a", crate::task_progress::TaskProgressStatus::Waiting),
            (
                "task-b",
                crate::task_progress::TaskProgressStatus::Completed,
            ),
        ] {
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: session_id.to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status,
                        intent_summary: Some(format!("summary-{session_id}")),
                        verification_state: None,
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 123,
                    },
                ),
            })
            .expect("append task progress event");
        }

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
        )
        .expect("tasks_list outcome");

        assert_eq!(outcome.payload["tool"], "tasks_list");
        assert_eq!(outcome.payload["matched_count"], 2);
        assert_eq!(
            outcome.payload["tasks"]
                .as_array()
                .expect("tasks array")
                .len(),
            2
        );
    }

    #[test]
    fn tasks_list_filters_stable_only_and_task_state() {
        let config = isolated_memory_config("tasks-list-filters-stable");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for (session_id, status) in [
            (
                "task-active",
                crate::task_progress::TaskProgressStatus::Active,
            ),
            (
                "task-waiting",
                crate::task_progress::TaskProgressStatus::Waiting,
            ),
            (
                "task-completed",
                crate::task_progress::TaskProgressStatus::Completed,
            ),
        ] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: session_id.to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status,
                        intent_summary: Some(session_id.to_owned()),
                        verification_state: None,
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 123,
                    },
                ),
            })
            .expect("append task progress event");
        }

        let stable_only = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_list".to_owned(),
                payload: json!({
                    "stable_only": true
                }),
            },
            "root-session",
            &config,
        )
        .expect("stable tasks_list outcome");
        assert_eq!(stable_only.payload["matched_count"], 2);

        let waiting_only = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_list".to_owned(),
                payload: json!({
                    "task_state": "waiting"
                }),
            },
            "root-session",
            &config,
        )
        .expect("waiting tasks_list outcome");
        assert_eq!(waiting_only.payload["matched_count"], 1);
        assert_eq!(waiting_only.payload["tasks"][0]["task_id"], "task-waiting");
    }

    #[test]
    fn tasks_search_matches_summary_and_state_filters() {
        let config = isolated_memory_config("tasks-search");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        for (session_id, summary, status) in [
            (
                "task-alpha",
                "refresh approval queue",
                crate::task_progress::TaskProgressStatus::Waiting,
            ),
            (
                "task-beta",
                "rebuild search index",
                crate::task_progress::TaskProgressStatus::Completed,
            ),
        ] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create root");
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: session_id.to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status,
                        intent_summary: Some(summary.to_owned()),
                        verification_state: None,
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 1,
                    },
                ),
            })
            .expect("append task progress event");
        }

        let summary_match = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_search".to_owned(),
                payload: json!({
                    "query": "approval",
                    "max_results": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("tasks_search outcome");

        assert_eq!(summary_match.payload["tool"], "tasks_search");
        assert_eq!(summary_match.payload["matched_count"], 1);
        assert_eq!(summary_match.payload["tasks"][0]["task_id"], "task-alpha");

        let state_match = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_search".to_owned(),
                payload: json!({
                    "query": "task",
                    "task_state": "completed",
                    "max_results": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("tasks_search filtered outcome");

        assert_eq!(state_match.payload["matched_count"], 1);
        assert_eq!(state_match.payload["tasks"][0]["task_id"], "task-beta");
    }

    #[test]
    fn task_surfaces_deduplicate_shared_canonical_task_ids_to_latest_owner_session() {
        let config = isolated_memory_config("task-deduplicate-latest-owner");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");

        for session_id in ["owner-old", "owner-new"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }

        for (session_id, summary, updated_at) in [
            ("owner-old", "legacy owner", 10),
            ("owner-new", "latest owner", 20),
        ] {
            repo.append_event(NewSessionEvent {
                session_id: session_id.to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some(session_id.to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: "task-shared".to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Waiting,
                        intent_summary: Some(summary.to_owned()),
                        verification_state: Some(
                            crate::task_progress::TaskVerificationState::Pending,
                        ),
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at,
                    },
                ),
            })
            .expect("append task progress event");
        }

        let task_status = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "task_status".to_owned(),
                payload: json!({
                    "task_id": "task-shared"
                }),
            },
            "root-session",
            &config,
        )
        .expect("task_status outcome");
        assert_eq!(task_status.payload["task_id"], "task-shared");
        assert_eq!(task_status.payload["owner_session_id"], "owner-new");
        assert_eq!(task_status.payload["task_session_id"], "owner-new");
        assert_eq!(task_status.payload["task_session_count"], 2);
        assert_eq!(
            task_status.payload["task_sessions"][0]["task_session_id"],
            "owner-old"
        );
        assert_eq!(
            task_status.payload["task_sessions"][0]["is_current_owner"],
            false
        );
        assert_eq!(
            task_status.payload["task_sessions"][1]["task_session_id"],
            "owner-new"
        );
        assert_eq!(
            task_status.payload["task_sessions"][1]["is_current_owner"],
            true
        );
        assert_eq!(
            task_status.payload["task_progress"]["intent_summary"],
            "latest owner"
        );

        let tasks_list = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_list".to_owned(),
                payload: json!({}),
            },
            "root-session",
            &config,
        )
        .expect("tasks_list outcome");
        assert_eq!(tasks_list.payload["matched_count"], 1);
        assert_eq!(tasks_list.payload["tasks"][0]["task_id"], "task-shared");
        assert_eq!(
            tasks_list.payload["tasks"][0]["owner_session_id"],
            "owner-new"
        );
        assert_eq!(
            tasks_list.payload["tasks"][0]["intent_summary"],
            "latest owner"
        );

        let tasks_search = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_search".to_owned(),
                payload: json!({
                    "query": "task-shared",
                    "max_results": 10
                }),
            },
            "root-session",
            &config,
        )
        .expect("tasks_search outcome");
        assert_eq!(tasks_search.payload["matched_count"], 1);
        assert_eq!(tasks_search.payload["tasks"][0]["task_id"], "task-shared");
        assert_eq!(
            tasks_search.payload["tasks"][0]["owner_session_id"],
            "owner-new"
        );
    }

    #[test]
    fn load_session_workflow_record_propagates_unexpected_lineage_lookup_failures() {
        let config = isolated_memory_config("session-workflow-lineage-errors");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");

        let session = repo
            .load_session_summary_with_legacy_fallback("root-session")
            .expect("load session summary")
            .expect("root session summary");

        let db_path = config
            .sqlite_path
            .as_ref()
            .expect("sqlite path for session tools test");
        let conn = rusqlite::Connection::open(db_path).expect("open sqlite db");
        conn.execute("DROP TABLE sessions", [])
            .expect("drop sessions table");

        let error = super::load_session_workflow_record(&repo, &session, None)
            .expect_err("unexpected lineage lookup failures should surface");

        assert!(
            error.contains("no such table: sessions"),
            "expected sqlite lineage lookup failure, got: {error}"
        );
    }

    #[test]
    fn optional_lineage_lookup_only_degrades_expected_gap_errors() {
        let broken = super::optional_lineage_lookup::<usize>(Err(
            "session_lineage_broken: missing parent row for `child-session`".to_owned(),
        ))
        .expect("broken lineage should degrade to missing");
        assert_eq!(broken, None);

        let cycle = super::optional_lineage_lookup::<usize>(Err(
            "session_lineage_cycle_detected: `child-session` reappeared".to_owned(),
        ))
        .expect("cycle lineage should degrade to missing");
        assert_eq!(cycle, None);

        let error = super::optional_lineage_lookup::<usize>(Err(
            "query sessions failed: database is locked".to_owned(),
        ))
        .expect_err("unexpected lineage lookup failures should not be swallowed");
        assert_eq!(error, "query sessions failed: database is locked");
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
            set.payload["policy"]["visible_requested_tool_ids"],
            json!(["agent", "tool.search"])
        );
        assert_eq!(
            set.payload["policy"]["effective_tool_ids"],
            json!(["session_status", "tool.search"])
        );
        assert_eq!(
            set.payload["policy"]["visible_effective_tool_ids"],
            json!(["agent", "tool.search"])
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
            status.payload["policy"]["visible_requested_tool_ids"],
            json!(["agent", "tool.search"])
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
                    app_id: Some(loong_contracts::SecretRef::Inline(
                        "test-feishu-app-id".to_owned(),
                    )),
                    app_secret: Some(loong_contracts::SecretRef::Inline(
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
    fn session_status_surfaces_latest_provider_failover_diagnostics() {
        let config = isolated_memory_config("session-status-provider-failover");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "trust_provider_failover".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "source": "provider_runtime",
                "binding": "kernel",
                "provider_id": "openai",
                "provider_failover": {
                    "reason": "rate_limited",
                    "stage": "status_failure",
                    "model": "gpt-4o",
                    "attempt": 2,
                    "max_attempts": 3,
                    "status_code": 429,
                    "request_id": "req-123"
                }
            }),
        })
        .expect("append provider failover event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["diagnostics"]["latest_provider_failover"]["provider_id"],
            "openai"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["latest_provider_failover"]["reason"],
            "rate_limited"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["latest_provider_failover"]["model"],
            "gpt-4o"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["latest_provider_failover"]["status_code"],
            429
        );
        let attention_hints = outcome.payload["diagnostics"]["attention_hints"]
            .as_array()
            .expect("attention_hints array");
        assert!(
            attention_hints.iter().any(|hint| {
                hint.as_str().is_some_and(|hint| {
                    hint.contains("provider_failover_present")
                        && hint.contains("reason=rate_limited")
                        && hint.contains("request_id=req-123")
                })
            }),
            "expected provider failover attention hint, got: {attention_hints:?}"
        );
    }

    #[test]
    fn session_status_recommends_session_recover_for_overdue_async_child() {
        let config = isolated_memory_config("session-status-recover-recommendation");
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
            outcome.payload["diagnostics"]["recommended_action"]["tool_name"],
            "session_recover"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["kind"],
            "queued_async_overdue_marked_failed"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["source"],
            "session_recover_plan"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["requires_mutation"],
            true
        );
    }

    #[test]
    fn session_status_recommends_resume_recipe_when_recover_plan_is_unavailable() {
        let config = isolated_memory_config("session-status-resume-recipe-recommendation");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "root-session".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Wait for the durable task to settle".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                        recommended_tool: "session_wait".to_owned(),
                        session_id: "root-session".to_owned(),
                        note: Some("Wait for the terminal transition.".to_owned()),
                    }),
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "root-session"
                }),
            },
            "root-session",
            &config,
        )
        .expect("session_status outcome");

        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["tool_name"],
            "session_wait"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["kind"],
            "follow_resume_recipe"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["source"],
            "task_progress_resume_recipe"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["task_status"],
            "waiting"
        );
        assert_eq!(
            outcome.payload["diagnostics"]["recommended_action"]["requires_mutation"],
            false
        );
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
        let frozen_error_code =
            outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"]
                .as_str()
                .expect("queued frozen error code");
        assert!(
            frozen_error_code.starts_with("delegate_async_queued_overdue_marked_failed:"),
            "unexpected queued frozen error code: {frozen_error_code}"
        );
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
        let frozen_error_code =
            outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"]
                .as_str()
                .expect("running frozen error code");
        assert!(
            frozen_error_code.starts_with("delegate_async_running_overdue_marked_failed:"),
            "unexpected running frozen error code: {frozen_error_code}"
        );
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
        assert_eq!(outcome.payload["workflow"]["phase"], "cancelled");
        assert_eq!(outcome.payload["terminal_outcome_state"], "present");
        assert_eq!(outcome.payload["terminal_outcome"]["status"], "error");
        assert_eq!(
            outcome.payload["terminal_outcome"]["frozen_result"]["content"]["error"]["code"],
            "delegate_cancelled: operator_requested"
        );
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
        let queued_outcome = repo
            .load_terminal_outcome("queued-child")
            .expect("load queued outcome")
            .expect("queued outcome row");
        assert!(queued_outcome.frozen_result.is_some());
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
                "profile": "research",
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

        assert_eq!(outcome.payload["delegate_lifecycle"]["profile"], "research");
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
        assert_eq!(outcome.payload["subagent"]["session_id"], "child-session");
        assert_eq!(outcome.payload["subagent_identity"]["nickname"], "Child");
        assert_eq!(
            outcome.payload["subagent_contract"]["identity"]["nickname"],
            "Child"
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
        append_session_turn_direct("delegate:legacy-child", "user", "hello", &config)
            .expect("append user turn");
        append_session_turn_direct("delegate:legacy-child", "assistant", "done", &config)
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
                frozen_result: None,
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
                    frozen_result: None,
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
                    frozen_result: None,
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

    #[tokio::test]
    async fn session_wait_wakes_when_parent_mailbox_receives_delegate_result() {
        let config = isolated_memory_config("session-wait-mailbox-wake");
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

        let config_for_completion = config.clone();
        let completion = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
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
                    frozen_result: None,
                },
            )
            .expect("finalize child");

            let mailbox = mailbox_for_session("root-session");
            let send_result = mailbox.send(InterAgentMessage {
                author: AgentPath::root(),
                recipient: AgentPath::root(),
                content: MailboxContent::DelegateResult {
                    session_id: "child-session".to_owned(),
                    frozen_result: json!({
                        "status": "ok"
                    }),
                },
                trigger_turn: true,
            });
            assert!(send_result.is_ok());
        });

        let wait_timeout_ms = 1_000_u64;
        let poll_interval_ms = 10_usize;
        let outcome = wait_for_single_session_with_policies(
            "child-session",
            "root-session",
            &config,
            &ToolConfig::default(),
            None,
            wait_timeout_ms,
            poll_interval_ms,
        )
        .await
        .expect("session_wait outcome");
        completion.await.expect("completion task");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["wait_status"], "completed");
        assert_eq!(outcome.payload["session"]["state"], "completed");
    }

    #[tokio::test]
    async fn task_wait_wakes_when_canonical_task_owner_session_completes() {
        let config = isolated_memory_config("task-wait-mailbox-wake");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "task-owner".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Task Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "task-owner".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-owner".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Active,
                    intent_summary: Some("Mailbox wake for canonical task".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 1,
                },
            ),
        })
        .expect("append task progress event");

        let config_for_completion = config.clone();
        let completion = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
            repo.finalize_session_terminal(
                "task-owner",
                FinalizeSessionTerminalRequest {
                    state: SessionState::Completed,
                    last_error: None,
                    event_kind: "delegate_completed".to_owned(),
                    actor_session_id: Some("task-owner".to_owned()),
                    event_payload_json: json!({
                        "result": "ok"
                    }),
                    outcome_status: "ok".to_owned(),
                    outcome_payload_json: json!({
                        "child_session_id": "task-owner",
                        "result": "ok"
                    }),
                    frozen_result: None,
                },
            )
            .expect("finalize task");

            let mailbox = mailbox_for_session("task-owner");
            let send_result = mailbox.send(InterAgentMessage {
                author: AgentPath::root(),
                recipient: AgentPath::root(),
                content: MailboxContent::DelegateResult {
                    session_id: "task-owner".to_owned(),
                    frozen_result: json!({
                        "status": "ok"
                    }),
                },
                trigger_turn: true,
            });
            assert!(send_result.is_ok());
        });

        let outcome = crate::tools::wait_for_task_with_config(
            json!({
                "task_id": "task-root",
                "timeout_ms": 1_000
            }),
            "task-owner",
            &config,
            &ToolConfig::default(),
        )
        .await
        .expect("task_wait outcome");
        completion.await.expect("completion task");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool"], "task_wait");
        assert_eq!(outcome.payload["task_id"], "task-root");
        assert_eq!(outcome.payload["owner_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_count"], 1);
        assert_eq!(
            outcome.payload["task_sessions"][0]["task_session_id"],
            "task-owner"
        );
        assert_eq!(outcome.payload["wait_status"], "completed");
        assert_eq!(outcome.payload["task_state"], "completed");
        assert_eq!(outcome.payload["task_is_stable"], true);
    }

    #[tokio::test]
    async fn task_wait_returns_immediately_for_waiting_canonical_task_state() {
        let config = isolated_memory_config("task-wait-waiting-state");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "task-owner".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Task Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        repo.append_event(NewSessionEvent {
            session_id: "task-owner".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-owner".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Await approval".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: vec![crate::task_progress::TaskActiveHandleRecord {
                        handle_kind: "approval_gate".to_owned(),
                        handle_id: "task-owner".to_owned(),
                        state: "waiting".to_owned(),
                        last_event_at: Some(123),
                        stop_condition: "approval_decision".to_owned(),
                    }],
                    resume_recipe: Some(crate::task_progress::TaskResumeRecipeRecord {
                        recommended_tool: "task_status".to_owned(),
                        session_id: "task-owner".to_owned(),
                        note: Some("Inspect task status for the approval gate.".to_owned()),
                    }),
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress event");

        let started_at = Instant::now();
        let outcome = crate::tools::wait_for_task_with_config(
            json!({
                "task_id": "task-root",
                "timeout_ms": 1_000
            }),
            "task-owner",
            &config,
            &ToolConfig::default(),
        )
        .await
        .expect("task_wait outcome");
        let immediate_resolution_budget = Duration::from_millis(500);

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["wait_status"], "waiting");
        assert_eq!(outcome.payload["owner_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_id"], "task-owner");
        assert_eq!(outcome.payload["task_session_count"], 1);
        assert_eq!(
            outcome.payload["task_sessions"][0]["task_session_id"],
            "task-owner"
        );
        assert_eq!(outcome.payload["task_state"], "waiting");
        assert_eq!(outcome.payload["task_is_stable"], true);
        assert_eq!(outcome.payload["continuation"]["state"], "waiting");
        assert_eq!(outcome.payload["continuation"]["is_terminal"], false);
        assert!(
            started_at.elapsed() < immediate_resolution_budget,
            "waiting task state should resolve without waiting for terminal session state"
        );
    }

    #[tokio::test]
    async fn session_wait_waiting_state_exposes_generic_continuation_metadata() {
        let config = isolated_memory_config("session-wait-continuation");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: Some("Child".to_owned()),
            state: SessionState::Running,
        })
        .expect("create child");

        let outcome = crate::tools::wait_for_session_with_config(
            json!({
                "session_id": "child-session",
                "timeout_ms": 100
            }),
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .await
        .expect("session_wait outcome");

        assert_eq!(outcome.payload["wait_status"], "timeout");
        assert_eq!(outcome.payload["continuation"]["state"], "timeout");
        assert_eq!(outcome.payload["continuation"]["is_terminal"], false);
    }

    #[tokio::test]
    async fn task_wait_follows_latest_owner_session_for_reassigned_task() {
        let config = isolated_memory_config("task-wait-reassigned-owner");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for session_id in ["owner-old", "owner-new"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }
        repo.append_event(NewSessionEvent {
            session_id: "owner-old".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-old".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Active,
                    intent_summary: Some("Initial owner".to_owned()),
                    verification_state: Some(
                        crate::task_progress::TaskVerificationState::NotStarted,
                    ),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 10,
                },
            ),
        })
        .expect("append old owner task progress");

        let config_for_completion = config.clone();
        let completion = tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let repo = SessionRepository::new(&config_for_completion).expect("completion repo");
            repo.append_event(NewSessionEvent {
                session_id: "owner-new".to_owned(),
                event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
                actor_session_id: Some("owner-new".to_owned()),
                payload_json: crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: "task-root".to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Completed,
                        intent_summary: Some("Reassigned owner".to_owned()),
                        verification_state: Some(
                            crate::task_progress::TaskVerificationState::Passed,
                        ),
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 20,
                    },
                ),
            })
            .expect("append new owner task progress");

            let mailbox = mailbox_for_session("root-session");
            let send_result = mailbox.send(InterAgentMessage {
                author: AgentPath::root(),
                recipient: AgentPath::root(),
                content: MailboxContent::DelegateResult {
                    session_id: "owner-new".to_owned(),
                    frozen_result: json!({
                        "status": "ok"
                    }),
                },
                trigger_turn: true,
            });
            assert!(send_result.is_ok());
        });

        let outcome = crate::tools::wait_for_task_with_config(
            json!({
                "task_id": "task-root",
                "timeout_ms": 1_000
            }),
            "root-session",
            &config,
            &ToolConfig::default(),
        )
        .await
        .expect("task_wait outcome");
        completion.await.expect("completion task");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["task_id"], "task-root");
        assert_eq!(outcome.payload["owner_session_id"], "owner-new");
        assert_eq!(outcome.payload["task_session_id"], "owner-new");
        let task_sessions = outcome.payload["task_sessions"]
            .as_array()
            .expect("task sessions");
        assert_eq!(outcome.payload["task_session_count"], 2);
        assert_eq!(task_sessions.len(), 2);
        assert_eq!(task_sessions[0]["task_session_id"], "owner-old");
        assert_eq!(task_sessions[0]["is_current_owner"], false);
        assert_eq!(task_sessions[1]["task_session_id"], "owner-new");
        assert_eq!(task_sessions[1]["is_current_owner"], true);
        assert_eq!(outcome.payload["wait_status"], "completed");
        assert_eq!(outcome.payload["task_state"], "completed");
        assert_eq!(outcome.payload["task_is_stable"], true);
    }

    #[test]
    fn tasks_list_filters_by_task_state_and_stability() {
        let config = isolated_memory_config("tasks-list-filters-visible");
        let repo = SessionRepository::new(&config).expect("repository");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        })
        .expect("create root");
        for session_id in ["task-active", "task-waiting"] {
            repo.create_session(NewSessionRecord {
                session_id: session_id.to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some(session_id.to_owned()),
                state: SessionState::Running,
            })
            .expect("create child");
        }
        repo.append_event(NewSessionEvent {
            session_id: "task-active".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-active".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-active".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Active,
                    intent_summary: Some("Active task".to_owned()),
                    verification_state: Some(
                        crate::task_progress::TaskVerificationState::NotStarted,
                    ),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 100,
                },
            ),
        })
        .expect("append active task progress event");
        repo.append_event(NewSessionEvent {
            session_id: "task-waiting".to_owned(),
            event_kind: crate::task_progress::TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("task-waiting".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-waiting".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: Some("Waiting task".to_owned()),
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 101,
                },
            ),
        })
        .expect("append waiting task progress event");

        let outcome = execute_session_tool_with_config(
            ToolCoreRequest {
                tool_name: "tasks_list".to_owned(),
                payload: json!({
                    "stable_only": true,
                    "task_state": "waiting"
                }),
            },
            "root-session",
            &config,
        )
        .expect("tasks_list outcome");

        assert_eq!(outcome.payload["tool"], "tasks_list");
        assert_eq!(outcome.payload["matched_count"], 1);
        assert_eq!(outcome.payload["tasks"][0]["task_id"], "task-waiting");
        assert_eq!(outcome.payload["tasks"][0]["task_state"], "waiting");
        assert_eq!(outcome.payload["tasks"][0]["task_is_stable"], true);
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
