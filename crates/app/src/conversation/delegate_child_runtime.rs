#[cfg(feature = "memory-sqlite")]
use std::any::Any;
#[cfg(feature = "memory-sqlite")]
use std::future::Future;
#[cfg(feature = "memory-sqlite")]
use std::panic::AssertUnwindSafe;

#[cfg(feature = "memory-sqlite")]
use futures_util::FutureExt;
#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::{AuditEventKind, ExecutionPlane, PlaneTier};
#[cfg(feature = "memory-sqlite")]
use serde_json::{Value, json};
#[cfg(feature = "memory-sqlite")]
use tokio::runtime::Handle;

#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::recovery::{RECOVERY_EVENT_KIND, build_terminal_finalize_recovery_payload};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    FinalizeSessionTerminalRequest, SessionRepository, SessionState,
    TransitionSessionWithEventIfCurrentRequest,
};

#[cfg(feature = "memory-sqlite")]
use super::persistence::persist_conversation_event;
#[cfg(feature = "memory-sqlite")]
use super::runtime::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner, ConversationRuntime};
#[cfg(feature = "memory-sqlite")]
use super::runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
#[cfg(feature = "memory-sqlite")]
use super::subagent::{ConstrainedSubagentExecution, DelegateBuiltinProfile};
#[cfg(feature = "memory-sqlite")]
use super::workspace_isolation::DelegateWorkspaceCleanupResult;

#[cfg(feature = "memory-sqlite")]
const DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS: usize = 200;

#[cfg(feature = "memory-sqlite")]
pub(super) async fn emit_async_delegate_child_queued_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    timeout_seconds: u64,
    workspace_root: Option<&std::path::Path>,
    binding: ConversationRuntimeBinding<'_>,
) {
    emit_delegate_child_projection_event(
        runtime,
        parent_session_id,
        "delegate_child_queued",
        json!({
            "child_session_id": child_session_id,
            "label": child_label,
            "profile": profile.map(DelegateBuiltinProfile::as_str),
            "mode": "async",
            "phase": "queued",
            "isolation": isolation.as_str(),
            "timeout_seconds": timeout_seconds,
            "workspace_root": workspace_root.map(|workspace_root| workspace_root.display().to_string()),
        }),
        binding,
    )
    .await;
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn emit_async_delegate_child_terminal_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    phase: &'static str,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    duration_ms: u64,
    turn_count: Option<usize>,
    error: Option<&str>,
    final_output: Option<&str>,
    workspace_root: Option<&std::path::Path>,
    workspace_retained: Option<bool>,
    binding: ConversationRuntimeBinding<'_>,
) {
    let payload = async_delegate_child_terminal_event_payload(
        child_session_id,
        child_label,
        profile,
        phase,
        isolation,
        duration_ms,
        turn_count,
        error,
        final_output,
        workspace_root,
        workspace_retained,
    );
    emit_delegate_child_projection_event(
        runtime,
        parent_session_id,
        "delegate_child_terminal",
        payload,
        binding,
    )
    .await;
}

#[cfg(feature = "memory-sqlite")]
fn async_delegate_child_terminal_event_payload(
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    phase: &'static str,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    duration_ms: u64,
    turn_count: Option<usize>,
    error: Option<&str>,
    final_output: Option<&str>,
    workspace_root: Option<&std::path::Path>,
    workspace_retained: Option<bool>,
) -> Value {
    json!({
        "child_session_id": child_session_id,
        "label": child_label,
        "profile": profile.map(DelegateBuiltinProfile::as_str),
        "mode": "async",
        "phase": phase,
        "isolation": isolation.as_str(),
        "duration_ms": duration_ms,
        "turn_count": turn_count,
        "error": error,
        "final_output_preview": final_output.map(truncate_delegate_child_output_preview),
        "workspace_root": workspace_root.map(|workspace_root| workspace_root.display().to_string()),
        "workspace_retained": workspace_retained,
    })
}

#[cfg(feature = "memory-sqlite")]
async fn emit_delegate_child_projection_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    event_name: &str,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) {
    let _ =
        persist_conversation_event(runtime, parent_session_id, event_name, payload, binding).await;
    if let Some(ctx) = binding.kernel_context() {
        let _ = ctx.kernel.record_audit_event(
            Some(ctx.agent_id()),
            AuditEventKind::PlaneInvoked {
                pack_id: ctx.pack_id().to_owned(),
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter: "conversation.delegate_child".to_owned(),
                delegated_core_adapter: None,
                operation: format!("conversation.delegate_child.{event_name}"),
                required_capabilities: Vec::new(),
            },
        );
    }
}

#[cfg(feature = "memory-sqlite")]
async fn persist_delegate_child_projection_event_without_runtime(
    memory_config: &MemoryRuntimeConfig,
    parent_session_id: &str,
    event_name: &str,
    payload: Value,
    _binding: &OwnedConversationRuntimeBinding,
) -> Result<(), String> {
    let repo = SessionRepository::new(memory_config)?;
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: parent_session_id.to_owned(),
        event_kind: event_name.to_owned(),
        actor_session_id: None,
        payload_json: payload,
    })?;
    Ok(())
}

#[cfg(feature = "memory-sqlite")]
fn truncate_delegate_child_output_preview(value: &str) -> String {
    let mut preview = value.trim().to_owned();
    let char_count = preview.chars().count();
    if char_count <= DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS {
        return preview;
    }

    preview = preview
        .chars()
        .take(DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS)
        .collect();
    preview.push_str("...");
    preview
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn inject_delegate_workspace_metadata(
    outcome: &mut loongclaw_contracts::ToolCoreOutcome,
    execution: &ConstrainedSubagentExecution,
    cleanup: Option<&DelegateWorkspaceCleanupResult>,
    cleanup_error: Option<String>,
) {
    let Some(outcome_payload) = outcome.payload.as_object_mut() else {
        return;
    };
    outcome_payload.insert("isolation".to_owned(), json!(execution.isolation.as_str()));
    if let Some(workspace_root) = execution.workspace_root.as_ref() {
        outcome_payload.insert(
            "workspace_root".to_owned(),
            json!(workspace_root.display().to_string()),
        );
    }
    if let Some(cleanup) = cleanup {
        outcome_payload.insert("workspace_retained".to_owned(), json!(cleanup.retained));
    }
    if let Some(cleanup_error) = cleanup_error {
        outcome_payload.insert("workspace_cleanup_error".to_owned(), json!(cleanup_error));
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn split_delegate_workspace_cleanup(
    cleanup: Result<Option<DelegateWorkspaceCleanupResult>, String>,
) -> (Option<DelegateWorkspaceCleanupResult>, Option<String>) {
    match cleanup {
        Ok(cleanup) => (cleanup, None),
        Err(cleanup_error) => (None, Some(cleanup_error)),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn mark_delegate_workspace_cleanup_owned_by_child(
    workspace_cleanup_owned_by_child: Option<&std::sync::atomic::AtomicBool>,
) {
    let Some(workspace_cleanup_owned_by_child) = workspace_cleanup_owned_by_child else {
        return;
    };
    workspace_cleanup_owned_by_child.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(feature = "memory-sqlite")]
async fn persist_async_delegate_spawn_failure_projection(
    memory_config: &MemoryRuntimeConfig,
    parent_session_id: &str,
    child_session_id: &str,
    label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    error: &str,
    binding: &OwnedConversationRuntimeBinding,
) -> Result<(), String> {
    let workspace_retained =
        load_async_delegate_spawn_failure_workspace_retained(memory_config, child_session_id)?;
    let event_payload = async_delegate_child_terminal_event_payload(
        child_session_id,
        label,
        profile,
        "failed",
        execution.isolation,
        0,
        None,
        Some(error),
        None,
        execution.workspace_root.as_deref(),
        workspace_retained,
    );
    persist_delegate_child_projection_event_without_runtime(
        memory_config,
        parent_session_id,
        "delegate_child_terminal",
        event_payload,
        binding,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
fn load_async_delegate_spawn_failure_workspace_retained(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
) -> Result<Option<bool>, String> {
    let repo = SessionRepository::new(memory_config)?;
    let terminal_outcome = repo.load_terminal_outcome(child_session_id)?;
    let workspace_retained = terminal_outcome.and_then(|terminal_outcome| {
        terminal_outcome
            .payload_json
            .get("workspace_retained")
            .and_then(Value::as_bool)
    });
    Ok(workspace_retained)
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
pub(super) fn spawn_async_delegate_detached(
    runtime_handle: Handle,
    memory_config: MemoryRuntimeConfig,
    spawner: std::sync::Arc<dyn AsyncDelegateSpawner>,
    request: AsyncDelegateSpawnRequest,
) {
    let child_session_id = request.child_session_id.clone();
    let parent_session_id = request.parent_session_id.clone();
    let label = request.label.clone();
    let profile = request.profile;
    let execution = request.execution.clone();
    let binding = request.binding.clone();
    runtime_handle.spawn(async move {
        let spawn_failure = match AssertUnwindSafe(spawner.spawn(request))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(panic_payload) => Some(format_async_delegate_spawn_panic(panic_payload)),
        };
        if let Some(error) = spawn_failure {
            let finalize_result =
                crate::operator::delegate_runtime::finalize_async_delegate_spawn_failure_with_recovery(
                    &memory_config,
                    &child_session_id,
                    &parent_session_id,
                    label.clone(),
                    profile,
                    &execution,
                    error.clone(),
                );
            let projected_error = match finalize_result {
                Ok(()) => error.clone(),
                Err(ref finalize_error) => finalize_error.clone(),
            };
            if let Err(finalize_error) = finalize_result {
                tracing::warn!(
                    target: "loongclaw.conversation",
                    child_session_id,
                    parent_session_id,
                    error = %crate::observability::summarize_error(finalize_error.as_str()),
                    "delegate async spawn failure recovery did not fully finalize child state"
                );
            }
            if let Err(projection_error) = persist_async_delegate_spawn_failure_projection(
                &memory_config,
                &parent_session_id,
                &child_session_id,
                label.as_deref(),
                profile,
                &execution,
                projected_error.as_str(),
                &binding,
            )
            .await
            {
                tracing::warn!(
                    target: "loongclaw.conversation",
                    child_session_id,
                    parent_session_id,
                    error = %crate::observability::summarize_error(projection_error.as_str()),
                    "delegate async spawn failure projection could not be persisted to the parent session"
                );
            }
        }
    });
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn with_prepared_subagent_spawn_cleanup_if_kernel_bound<
    R: ConversationRuntime + ?Sized,
    F,
    Fut,
    T,
>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
    work: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    prepare_subagent_spawn_if_kernel_bound(runtime, parent_session_id, child_session_id, binding)
        .await?;
    let work_result = work().await;
    let notify_result = notify_subagent_ended_if_kernel_bound(
        runtime,
        parent_session_id,
        child_session_id,
        binding,
    )
    .await;
    match (work_result, notify_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(work_error), Ok(())) => Err(work_error),
        (Ok(_), Err(notify_error)) => {
            Err(format!("delegate_subagent_end_hook_failed: {notify_error}"))
        }
        (Err(work_error), Err(notify_error)) => Err(format!(
            "{work_error}; delegate_subagent_end_hook_failed: {notify_error}"
        )),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn prepare_subagent_spawn_if_kernel_bound<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<(), String> {
    let Some(kernel_ctx) = binding.kernel_context() else {
        return Ok(());
    };
    runtime
        .prepare_subagent_spawn(parent_session_id, child_session_id, kernel_ctx)
        .await
}

#[cfg(feature = "memory-sqlite")]
async fn notify_subagent_ended_if_kernel_bound<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<(), String> {
    let Some(kernel_ctx) = binding.kernel_context() else {
        return Ok(());
    };
    runtime
        .on_subagent_ended(parent_session_id, child_session_id, kernel_ctx)
        .await
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn finalize_delegate_child_terminal_with_recovery(
    repo: &SessionRepository,
    child_session_id: &str,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    let recovery_request = request.clone();
    match finalize_terminal_if_current_allowing_stale_state(
        repo,
        child_session_id,
        SessionState::Running,
        request,
    ) {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let recovery_error = format!("delegate_terminal_finalize_failed: {finalize_error}");
            match repo.transition_session_with_event_if_current(
                child_session_id,
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Running,
                    next_state: SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: recovery_request.actor_session_id.clone(),
                    event_payload_json: build_terminal_finalize_recovery_payload(
                        &recovery_request,
                        &recovery_error,
                    ),
                },
            ) {
                Ok(Some(_)) => Err(recovery_error),
                Ok(None) => {
                    delegate_terminal_recovery_skipped_error(repo, child_session_id, recovery_error)
                }
                Err(recovery_event_error) => match repo.update_session_state_if_current(
                    child_session_id,
                    SessionState::Running,
                    SessionState::Failed,
                    Some(recovery_error.clone()),
                ) {
                    Ok(Some(_)) => Err(format!(
                        "{recovery_error}; delegate_terminal_recovery_event_failed: {recovery_event_error}"
                    )),
                    Ok(None) => delegate_terminal_recovery_skipped_error(
                        repo,
                        child_session_id,
                        recovery_error,
                    ),
                    Err(mark_error) => Err(format!(
                        "{recovery_error}; delegate_terminal_recovery_failed: {mark_error}"
                    )),
                },
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn finalize_terminal_if_current_allowing_stale_state(
    repo: &SessionRepository,
    session_id: &str,
    expected_state: SessionState,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    match repo.finalize_session_terminal_if_current(session_id, expected_state, request)? {
        Some(_) => Ok(()),
        None => {
            if repo.load_session(session_id)?.is_some() {
                Ok(())
            } else {
                Err(format!("session `{session_id}` not found"))
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn delegate_terminal_recovery_skipped_error(
    repo: &SessionRepository,
    child_session_id: &str,
    recovery_error: String,
) -> Result<(), String> {
    let current_state = repo
        .load_session(child_session_id)?
        .map(|session| session.state.as_str().to_owned())
        .unwrap_or_else(|| "missing".to_owned());
    Err(format!(
        "{recovery_error}; delegate_terminal_recovery_skipped_from_state: {current_state}"
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn format_delegate_child_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_child_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };
    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_child_panic: {}", *message),
        Err(_) => "delegate_child_panic".to_owned(),
    }
}
