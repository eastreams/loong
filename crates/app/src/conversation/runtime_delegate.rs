use std::sync::Arc;

use loong_core::policy::context::PolicyContext;

use crate::conversation::DefaultLegacyToolDispatcher;
use crate::{Context, Session};
use async_trait::async_trait;

use super::super::subagent::{
    ConstrainedSubagentExecution, ConstrainedSubagentSpawnEventPayload, DelegateBuiltinProfile,
};
use super::super::{delegate_support, turn_coordinator};
use super::{LoongConfig, RuntimeSelfContinuity};

#[derive(Clone)]
pub struct AsyncDelegateSpawnRequest {
    /// Parent fallback owner retained for lifecycle hooks after detachment.
    legacy_tools: DefaultLegacyToolDispatcher,
    parent_session: Session,
    session: Session,
    task: String,
    canonical_task_id: Option<String>,
}

impl AsyncDelegateSpawnRequest {
    /// Bind detached work to one already-derived child Session and Runtime.
    ///
    /// Identity and execution limits are deliberately not constructor inputs:
    /// the atomically-derived child Session is their sole source, so callers
    /// cannot assemble a request whose lifecycle and execution evidence disagree.
    pub(crate) fn new(
        parent_context: &Context<'_>,
        legacy_tools: &DefaultLegacyToolDispatcher,
        session: Session,
        task: String,
        canonical_task_id: Option<String>,
    ) -> Result<Self, String> {
        if !std::ptr::eq(
            parent_context.runtime(),
            legacy_tools.execution_runtime().as_ref(),
        ) {
            return Err("async delegate fallback belongs to a different Runtime".to_owned());
        }
        if session.parent_session_id() != Some(parent_context.session().session_id()) {
            return Err("async delegate child Session does not belong to the caller".to_owned());
        }
        let execution = session
            .subagent_execution()
            .ok_or_else(|| "async delegate child Session has no execution anchor".to_owned())?;
        if session.baseline_capabilities() != &execution.capability_ceiling {
            return Err("async delegate execution does not match child capabilities".to_owned());
        }
        let parent_capabilities = parent_context.allowed_capabilities();
        let parent_session = parent_context
            .session()
            .clone()
            .narrow_capabilities(parent_capabilities.as_ref())?;
        Ok(Self {
            legacy_tools: legacy_tools.clone(),
            parent_session,
            session,
            task,
            canonical_task_id,
        })
    }

    #[must_use]
    pub fn child_session_id(&self) -> &str {
        self.session.session_id()
    }

    #[must_use]
    pub fn parent_session_id(&self) -> &str {
        self.parent_session.session_id()
    }

    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    #[must_use]
    pub fn canonical_task_id(&self) -> Option<&str> {
        self.canonical_task_id.as_deref()
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.session
            .resolved_subagent_identity()
            .and_then(|identity| identity.nickname.as_deref())
    }

    #[must_use]
    pub fn profile(&self) -> Option<DelegateBuiltinProfile> {
        self.session.profile
    }

    #[must_use]
    #[allow(
        clippy::expect_used,
        reason = "private constructors reject a child Session without this anchor, and the request cannot be deserialized or assembled by callers"
    )]
    pub fn execution(&self) -> &ConstrainedSubagentExecution {
        self.session
            .subagent_execution()
            .expect("AsyncDelegateSpawnRequest requires a child execution anchor")
    }

    #[must_use]
    pub fn runtime_self_continuity(&self) -> Option<&RuntimeSelfContinuity> {
        self.session.runtime_self_continuity.as_ref()
    }

    #[must_use]
    pub fn timeout_seconds(&self) -> u64 {
        self.execution().timeout_seconds
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    pub(crate) fn parent_session(&self) -> &Session {
        &self.parent_session
    }

    pub(crate) fn legacy_tools(&self) -> &DefaultLegacyToolDispatcher {
        &self.legacy_tools
    }

    /// Restore detached execution exclusively from persisted delegate evidence.
    ///
    /// The process payload identifies the child and its audit actor; capability,
    /// path, tool, profile, and timeout authority are rebuilt from the queued
    /// event and validated again by Session and request construction.
    #[cfg(feature = "memory-sqlite")]
    pub fn from_persisted_child(
        config: &LoongConfig,
        child_session_id: &str,
        agent_id: &str,
    ) -> Result<Self, String> {
        let memory_config =
            crate::session::store::session_store_config_from_memory_config_without_env_overrides(
                &config.memory,
            );
        let repo = crate::session::repository::SessionRepository::new(&memory_config)?;
        let event = repo
            .list_delegate_lifecycle_events(child_session_id)?
            .into_iter()
            .rev()
            .find(|event| {
                matches!(
                    event.event_kind.as_str(),
                    "delegate_queued" | "delegate_started"
                )
            })
            .ok_or_else(|| {
                format!("delegate session `{child_session_id}` has no persisted execution anchor")
            })?;
        let persisted: ConstrainedSubagentSpawnEventPayload =
            serde_json::from_value(event.payload_json).map_err(|error| {
                format!("decode persisted delegate execution for `{child_session_id}`: {error}")
            })?;

        let runtime = crate::runtime::bootstrap_runtime_with_config(config)?;
        let child_session = Session::from_config(
            runtime.as_ref(),
            config,
            child_session_id,
            agent_id,
            loong_contracts::GovernedSessionMode::MutatingCapable,
        )?;
        let parent_session_id = child_session.parent_session_id().ok_or_else(|| {
            format!("detached delegate `{child_session_id}` has no parent Session")
        })?;
        let parent_session = Session::from_config(
            runtime.as_ref(),
            config,
            parent_session_id,
            agent_id,
            loong_contracts::GovernedSessionMode::MutatingCapable,
        )?;
        let parent_context =
            Context::new(runtime.as_ref(), &parent_session).map_err(|error| error.to_string())?;
        let legacy_tools = DefaultLegacyToolDispatcher::with_config(
            Arc::clone(&runtime),
            &parent_session,
            memory_config,
            config.clone(),
        )?;

        Self::new(
            &parent_context,
            &legacy_tools,
            child_session,
            persisted.task,
            persisted.task_scope.map(|scope| scope.task_id),
        )
    }
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
    let execution = request.execution().clone();
    let AsyncDelegateSpawnRequest {
        legacy_tools,
        parent_session,
        session,
        task,
        canonical_task_id,
    } = request;

    let parent_session_id = parent_session.session_id().to_owned();
    let child_session_id = session.session_id().to_owned();
    let label = session
        .resolved_subagent_identity()
        .and_then(|identity| identity.nickname.clone());
    let profile = session.profile;
    let runtime_self_continuity = session.runtime_self_continuity.clone();
    let execution_timeout_seconds = execution.timeout_seconds;

    let memory_config =
        crate::session::store::session_store_config_from_memory_config_without_env_overrides(
            &config.memory,
        );
    let repo = crate::session::repository::SessionRepository::new(&memory_config)?;
    let parent_session =
        parent_session.rematerialize(legacy_tools.execution_runtime().as_ref(), config)?;
    let session = session.rematerialize(legacy_tools.execution_runtime().as_ref(), config)?;
    let parent_runtime = super::DefaultConversationRuntime::from_config_or_env(config)?;
    let child_legacy_tools = legacy_tools.for_session(&session)?;
    let child_runtime = super::DefaultConversationRuntime::from_config_or_env(config)?;
    let parent_context = Context::new(legacy_tools.execution_runtime(), &parent_session)
        .map_err(|error| error.to_string())?;
    let parent_mailbox = parent_session.mailbox().sender();
    let child_execution_runtime = Arc::clone(legacy_tools.execution_runtime());
    let child_session = session.clone();
    let child_session_id_for_spawn = child_session_id.clone();
    let parent_session_id_for_spawn = parent_session_id.clone();
    delegate_support::with_subagent_lifecycle(
        &parent_runtime,
        &child_session_id,
        &parent_context,
        move || async move {
            let child_context = Context::new(&child_execution_runtime, &child_session)
                .map_err(|error| error.to_string())?;
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
                &child_runtime,
                &child_context,
                &parent_mailbox,
                &child_legacy_tools,
                label,
                &task,
                profile,
                execution,
                execution_timeout_seconds,
            )
            .await;

            Ok(())
        },
    )
    .await?;

    Ok(())
}
