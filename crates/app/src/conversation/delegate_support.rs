#[cfg(feature = "memory-sqlite")]
use std::any::Any;
#[cfg(feature = "memory-sqlite")]
use std::future::Future;
#[cfg(feature = "memory-sqlite")]
use std::panic::AssertUnwindSafe;
#[cfg(feature = "memory-sqlite")]
use std::sync::Arc;

#[cfg(feature = "memory-sqlite")]
use futures_util::FutureExt;
#[cfg(feature = "memory-sqlite")]
use serde_json::Value;
#[cfg(feature = "memory-sqlite")]
use tokio::runtime::Handle;

#[cfg(feature = "memory-sqlite")]
use crate::Context;
use crate::config::LoongConfig;
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::next_delegate_child_depth;
#[cfg(feature = "memory-sqlite")]
use crate::session::frozen_result::capture_frozen_result;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{FinalizeSessionTerminalRequest, SessionRepository, SessionState};
use crate::session::store::SessionStoreConfig;

#[cfg(feature = "memory-sqlite")]
use super::announce::{DelegateAnnounceSettings, enqueue_delegate_result_announce};
#[cfg(feature = "memory-sqlite")]
use super::runtime::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner, ConversationRuntime};
#[cfg(feature = "memory-sqlite")]
#[cfg(feature = "memory-sqlite")]
use super::subagent::ConstrainedSubagentExecution;
#[cfg(feature = "memory-sqlite")]
use super::turn_coordinator::emit_async_delegate_child_terminal_event;

#[cfg(all(feature = "memory-sqlite", test))]
pub(crate) fn finalize_async_delegate_spawn_failure(
    memory_config: &SessionStoreConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    max_frozen_bytes: usize,
    error: String,
) -> Result<(), String> {
    crate::operator::delegate_runtime::finalize_async_delegate_spawn_failure(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        profile,
        execution,
        max_frozen_bytes,
        error,
    )
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &SessionStoreConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    max_frozen_bytes: usize,
    error: String,
) -> Result<(), String> {
    crate::operator::delegate_runtime::finalize_async_delegate_spawn_failure_with_recovery(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        profile,
        execution,
        max_frozen_bytes,
        error,
    )
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn format_async_delegate_spawn_panic(panic_payload: Box<dyn Any + Send>) -> String {
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
pub(crate) fn spawn_async_delegate_detached(
    runtime_handle: Handle,
    config: Arc<LoongConfig>,
    memory_config: SessionStoreConfig,
    spawner: Arc<dyn AsyncDelegateSpawner>,
    request: AsyncDelegateSpawnRequest,
    max_frozen_bytes: usize,
    announce_settings: DelegateAnnounceSettings,
) {
    let child_session_id = request.child_session_id().to_owned();
    let parent_session_id = request.parent_session_id().to_owned();
    let label = request.label().map(str::to_owned);
    let profile = request.profile();
    let execution = request.execution().clone();
    let execution_runtime = Arc::clone(request.legacy_tools().execution_runtime());
    let session = request.parent_session().clone();
    runtime_handle.spawn(async move {
        let spawn_failure = match AssertUnwindSafe(spawner.spawn(request))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(panic_payload) => Some(format_async_delegate_spawn_panic(panic_payload)),
        };

        let Some(error) = spawn_failure else {
            return;
        };

        let spawn_error = error.clone();
        let finalize_result = finalize_async_delegate_spawn_failure_with_recovery(
            &memory_config,
            &child_session_id,
            &parent_session_id,
            label.clone(),
            profile,
            &execution,
            max_frozen_bytes,
            error.clone(),
        );
        if let Err(finalize_error) = &finalize_result {
            tracing::warn!(
                child_session_id = %child_session_id,
                parent_session_id = %parent_session_id,
                spawn_error = %spawn_error,
                error = %finalize_error,
                "delegate async spawn failure finalize fell back with error"
            );
        }

        enqueue_delegate_result_announce_with_memory_config(
            memory_config.clone(),
            parent_session_id.clone(),
            child_session_id.clone(),
            announce_settings.clone(),
        );

        let runtime =
            crate::conversation::runtime::BoxedDefaultConversationRuntime::from_config_or_env(
                config.as_ref(),
            );
        match runtime {
            Ok(runtime) => {
                let context = match Context::new(&execution_runtime, &session) {
                    Ok(context) => context,
                    Err(error) => {
                        tracing::error!(
                            child_session_id = %child_session_id,
                            %error,
                            "cannot emit delegate terminal event through mismatched Runtime"
                        );
                        return;
                    }
                };
                emit_async_delegate_child_terminal_event(
                    &runtime,
                    &child_session_id,
                    label.as_deref(),
                    profile,
                    "failed",
                    execution.isolation,
                    0,
                    None,
                    Some(error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    None,
                    &context,
                )
                .await;
            }
            Err(runtime_error) => {
                tracing::warn!(
                    child_session_id = %child_session_id,
                    parent_session_id = %parent_session_id,
                    error = %runtime_error,
                    "delegate async spawn failure could not emit parent terminal projection"
                );
            }
        }
    });
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn enqueue_delegate_result_announce_for_parent(
    config: &LoongConfig,
    parent_session_id: &str,
    child_session_id: &str,
) {
    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
    let announce_settings = DelegateAnnounceSettings::from_config(config);

    enqueue_delegate_result_announce_with_memory_config(
        memory_config,
        parent_session_id.to_owned(),
        child_session_id.to_owned(),
        announce_settings,
    );
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn enqueue_delegate_result_announce_with_memory_config(
    memory_config: SessionStoreConfig,
    parent_session_id: String,
    child_session_id: String,
    announce_settings: DelegateAnnounceSettings,
) {
    enqueue_delegate_result_announce(
        memory_config,
        parent_session_id,
        child_session_id,
        announce_settings,
    );
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn next_delegate_child_depth_for_delegate(
    config: &LoongConfig,
    repo: &SessionRepository,
    session_context: &Context<'_>,
) -> Result<usize, String> {
    next_delegate_child_depth(
        repo,
        &session_context.session().session_id,
        config.tools.delegate.max_depth,
    )
}

#[cfg(feature = "memory-sqlite")]
/// Run one child lifecycle with hooks paired even when child work fails.
///
/// This helper exists because combining work and terminal-hook failures is a
/// single orchestration boundary; it does not adapt or hide Context authority.
pub(crate) async fn with_subagent_lifecycle<R: ConversationRuntime + ?Sized, F, Fut, T>(
    runtime: &R,
    child_session_id: &str,
    ctx: &Context<'_>,
    work: F,
) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    runtime
        .prepare_subagent_spawn(child_session_id, ctx)
        .await?;

    let work_result = work().await;
    let notify_result = runtime.on_subagent_ended(child_session_id, ctx).await;

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
pub(crate) fn finalize_delegate_child_terminal_with_recovery(
    repo: &SessionRepository,
    child_session_id: &str,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    crate::operator::delegate_runtime::finalize_delegate_child_terminal_with_recovery(
        repo,
        child_session_id,
        request,
    )
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn finalize_and_announce_delegate_child_terminal(
    config: &LoongConfig,
    repo: &SessionRepository,
    child_session_id: &str,
    parent_session_id: &str,
    outcome: &loong_contracts::ToolCoreOutcome,
    state: SessionState,
    last_error: Option<String>,
    event_kind: &str,
    event_payload_json: Value,
) -> Result<(), String> {
    let max_frozen_bytes = config.tools.delegate.max_frozen_bytes;
    let frozen_result = capture_frozen_result(outcome, max_frozen_bytes);

    let request = FinalizeSessionTerminalRequest {
        state,
        last_error,
        event_kind: event_kind.to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json,
        outcome_status: outcome.status.clone(),
        outcome_payload_json: outcome.payload.clone(),
        frozen_result: Some(frozen_result),
    };

    finalize_delegate_child_terminal_with_recovery(repo, child_session_id, request)?;
    enqueue_delegate_result_announce_for_parent(config, parent_session_id, child_session_id);

    Ok(())
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn format_delegate_child_panic(panic_payload: Box<dyn Any + Send>) -> String {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return format!("delegate_child_panic: {}", *message),
        Err(panic_payload) => panic_payload,
    };

    match panic_payload.downcast::<&'static str>() {
        Ok(message) => format!("delegate_child_panic: {}", *message),
        Err(_) => "delegate_child_panic".to_owned(),
    }
}
