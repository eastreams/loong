use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::super::runtime_binding::OwnedConversationRuntimeBinding;
use super::super::subagent::{ConstrainedSubagentExecution, DelegateBuiltinProfile};
use super::super::{delegate_support, turn_coordinator};
use super::{LoongConfig, RuntimeSelfContinuity, load_default_conversation_runtime};

#[derive(Clone)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub canonical_task_id: Option<String>,
    pub label: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub execution: ConstrainedSubagentExecution,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
    pub timeout_seconds: u64,
    pub binding: OwnedConversationRuntimeBinding,
}

impl AsyncDelegateSpawnRequest {
    pub fn runtime_self_continuity_json(&self) -> Result<Option<Value>, String> {
        let continuity = self.runtime_self_continuity.as_ref();
        let encoded_continuity =
            continuity
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| {
                    format!("serialize async delegate runtime-self continuity failed: {error}")
                })?;

        Ok(encoded_continuity)
    }
}

pub fn async_delegate_spawn_request_from_serialized_parts(
    child_session_id: String,
    parent_session_id: String,
    task: String,
    canonical_task_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    execution: ConstrainedSubagentExecution,
    runtime_self_continuity_json: Option<Value>,
    timeout_seconds: u64,
    binding: OwnedConversationRuntimeBinding,
) -> Result<AsyncDelegateSpawnRequest, String> {
    let runtime_self_continuity = runtime_self_continuity_json
        .map(serde_json::from_value::<RuntimeSelfContinuity>)
        .transpose()
        .map_err(|error| format!("parse async delegate runtime-self continuity failed: {error}"))?;
    let request = AsyncDelegateSpawnRequest {
        child_session_id,
        parent_session_id,
        task,
        canonical_task_id,
        label,
        profile,
        execution,
        runtime_self_continuity,
        timeout_seconds,
        binding,
    };

    Ok(request)
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub(super) struct DefaultAsyncDelegateSpawner {
    config: Arc<LoongConfig>,
}

#[cfg(feature = "memory-sqlite")]
impl DefaultAsyncDelegateSpawner {
    pub(super) fn new(config: &LoongConfig) -> Self {
        Self {
            config: Arc::new(config.clone()),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for DefaultAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        execute_async_delegate_spawn_request(self.config.as_ref(), request).await?;
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
pub async fn execute_async_delegate_spawn_request(
    config: &LoongConfig,
    request: AsyncDelegateSpawnRequest,
) -> Result<(), String> {
    let AsyncDelegateSpawnRequest {
        child_session_id,
        parent_session_id,
        task,
        canonical_task_id,
        label,
        profile,
        execution,
        runtime_self_continuity,
        timeout_seconds,
        binding,
    } = request;

    let execution_timeout_seconds = execution.timeout_seconds;

    if timeout_seconds != execution_timeout_seconds {
        return Err(format!(
            "async_delegate_timeout_mismatch: request timeout {} != execution timeout {}",
            timeout_seconds, execution_timeout_seconds
        ));
    }

    let memory_config =
        crate::session::store::session_store_config_from_memory_config_without_env_overrides(
            &config.memory,
        );
    let repo = crate::session::repository::SessionRepository::new(&memory_config)?;
    let runtime = load_default_conversation_runtime(config)?;
    let runtime_ref = &runtime;
    let child_session_id_for_spawn = child_session_id.clone();
    let parent_session_id_for_spawn = parent_session_id.clone();
    let borrowed_binding = binding.as_borrowed();
    let child_binding = binding.clone();

    delegate_support::with_prepared_subagent_spawn_cleanup_if_kernel_bound(
        runtime_ref,
        &parent_session_id,
        &child_session_id,
        borrowed_binding,
        move || async move {
            let event_payload_json = execution
                .spawn_payload_with_profile_and_runtime_self_continuity(
                    &task,
                    label.as_deref(),
                    profile,
                    runtime_self_continuity.as_ref(),
                    canonical_task_id.as_deref(),
                    Some(child_session_id_for_spawn.as_str()),
                );
            let transition_request =
                crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                    expected_state: crate::session::repository::SessionState::Ready,
                    next_state: crate::session::repository::SessionState::Running,
                    last_error: None,
                    event_kind: "delegate_started".to_owned(),
                    actor_session_id: Some(parent_session_id_for_spawn.clone()),
                    event_payload_json,
                };
            let started = repo.transition_session_with_event_if_current(
                &child_session_id_for_spawn,
                transition_request,
            )?;

            if started.is_none() {
                return Err(format!(
                    "async_delegate_spawn_skipped: session `{}` was not in Ready state",
                    child_session_id_for_spawn
                ));
            }

            let _ = turn_coordinator::run_started_delegate_child_turn_with_runtime(
                config,
                runtime_ref,
                &child_session_id_for_spawn,
                &parent_session_id_for_spawn,
                label,
                &task,
                profile,
                execution,
                execution_timeout_seconds,
                child_binding.as_borrowed(),
            )
            .await;

            Ok(())
        },
    )
    .await?;

    Ok(())
}
