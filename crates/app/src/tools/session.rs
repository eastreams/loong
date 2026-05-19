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
    NewSessionArtifactRecord, NewSessionRecord, NewSessionToolPolicyRecord, SessionArtifactKind,
    SessionArtifactRecord, SessionEventRecord, SessionHeadMode, SessionHeadRecord, SessionKind,
    SessionNodeRecord, SessionObservationRecord, SessionRepository, SessionState,
    SessionSummaryRecord, SessionTerminalOutcomeRecord, SessionToolPolicyRecord,
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
    pub tree: SessionTreeSnapshotRecord,
    pub subagent_contract: Option<ConstrainedSubagentContractView>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SessionObservationSnapshot {
    pub inspection: SessionInspectionSnapshot,
    pub tail_events: Vec<SessionEventRecord>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionTreeSnapshotRecord {
    pub(crate) heads: Vec<SessionHeadRecord>,
    pub(crate) active_path: Vec<SessionNodeRecord>,
    pub(crate) artifacts: Vec<SessionArtifactRecord>,
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

#[cfg(feature = "memory-sqlite")]
mod mutations;

#[cfg(feature = "memory-sqlite")]
mod projections;

#[cfg(feature = "memory-sqlite")]
use self::mutations::*;

#[cfg(feature = "memory-sqlite")]
use self::projections::*;

#[cfg(feature = "memory-sqlite")]
pub(crate) use self::projections::load_session_workflow_record;

#[cfg(test)]
mod session_tool_tests;

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
            "task_cancel" => execute_task_cancel(payload, current_session_id, config, tool_config),
            "task_recover" => {
                execute_task_recover(payload, current_session_id, config, tool_config)
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
            "session_search" => super::session_search::execute_session_search_with_policies(
                payload,
                current_session_id,
                config,
                tool_config,
            ),
            "session_heads" => {
                execute_session_heads(payload, current_session_id, config, tool_config)
            }
            "session_path" => {
                execute_session_path(payload, current_session_id, config, tool_config)
            }
            "session_children" => {
                execute_session_children(payload, current_session_id, config, tool_config)
            }
            "session_artifacts" => {
                execute_session_artifacts(payload, current_session_id, config, tool_config)
            }
            "session_status" => {
                execute_session_status(payload, current_session_id, config, tool_config)
            }
            "task_status" => execute_task_status(payload, current_session_id, config, tool_config),
            "session_create_checkpoint" => {
                execute_session_create_checkpoint(payload, current_session_id, config, tool_config)
            }
            "session_create_branch_summary" => execute_session_create_branch_summary(
                payload,
                current_session_id,
                config,
                tool_config,
            ),
            "session_fork_head" => {
                execute_session_fork_head(payload, current_session_id, config, tool_config)
            }
            "session_pin_head" => {
                execute_session_pin_head(payload, current_session_id, config, tool_config)
            }
            "session_set_active_head" => {
                execute_session_set_active_head(payload, current_session_id, config, tool_config)
            }
            "session_unpin_head" => {
                execute_session_unpin_head(payload, current_session_id, config, tool_config)
            }
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
        if matches!(canonical_tool_id.as_str(), "tool.search" | "tool.invoke") {
            return Err(format!(
                "session_tool_policy_set_invalid_tool_id: `{raw_tool_id}` is a legacy discovery wrapper and is not allowed in session tool policy"
            ));
        }
        let visible_tool_id = crate::tools::model_visible_tool_name(canonical_tool_id.as_str());
        if !base_tool_view.contains(&visible_tool_id) {
            return Err(format!(
                "session_tool_policy_set_invalid_tool_id: `{raw_tool_id}` is not available in session `{session_id}`"
            ));
        }
        normalized_tool_ids.insert(visible_tool_id.clone(), visible_tool_id);
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
