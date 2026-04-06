use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use loongclaw_contracts::Capability;
use serde_json::Value;

use crate::CliResult;
use crate::KernelContext;
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::{
    derive_subagent_profile_from_lineage, load_delegate_execution, resolve_delegate_child_contract,
};
use crate::runtime_self_continuity::{self, RuntimeSelfContinuity};
use crate::tools::runtime_config::ToolRuntimeNarrowing;
use crate::tools::{ToolView, delegate_child_tool_view_for_runtime_config_and_contract};

use super::super::memory;
use super::super::{config::LoongClawConfig, provider};
use super::context_engine::ContextArtifactKind;
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
use super::context_engine_registry::{
    DEFAULT_CONTEXT_ENGINE_ID, context_engine_id_from_env, describe_context_engine,
    list_context_engine_metadata, resolve_context_engine,
};
use super::prompt_orchestrator::seed_prompt_fragments_from_context;
use super::prompt_orchestrator::sync_prompt_fragments_into_context;
use super::runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
use super::subagent::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile,
};
use super::turn_engine::ProviderTurn;
use super::turn_middleware::{
    ConversationTurnMiddleware, TurnMiddlewareMetadata, builtin_turn_middlewares,
};
use super::turn_middleware_registry::{
    default_turn_middleware_ids, describe_turn_middlewares, list_turn_middleware_metadata,
    resolve_turn_middlewares, turn_middleware_ids_from_env,
};
use super::{PromptFragment, PromptLane};

#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    SessionKind, SessionRepository, SessionState, SessionToolPolicyRecord,
    TransitionSessionWithEventIfCurrentRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub tool_view: ToolView,
    pub runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub subagent_execution: Option<ConstrainedSubagentExecution>,
    pub subagent_contract: Option<ConstrainedSubagentContractView>,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

impl SessionContext {
    pub fn root_with_tool_view(session_id: impl Into<String>, tool_view: ToolView) -> Self {
        Self {
            session_id: normalize_session_id(session_id.into()),
            parent_session_id: None,
            tool_view,
            runtime_narrowing: None,
            subagent_execution: None,
            subagent_contract: None,
            runtime_self_continuity: None,
        }
    }

    pub fn child(
        session_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        tool_view: ToolView,
    ) -> Self {
        Self {
            session_id: normalize_session_id(session_id.into()),
            parent_session_id: Some(normalize_session_id(parent_session_id.into())),
            tool_view,
            runtime_narrowing: None,
            subagent_execution: None,
            subagent_contract: None,
            runtime_self_continuity: None,
        }
    }

    #[must_use]
    pub fn with_runtime_narrowing(mut self, runtime_narrowing: ToolRuntimeNarrowing) -> Self {
        if !runtime_narrowing.is_empty() {
            self.runtime_narrowing = Some(runtime_narrowing.clone());
            let contract = self.subagent_contract.take().unwrap_or_default();
            self.subagent_contract = Some(contract.with_runtime_narrowing(runtime_narrowing));
            self.synchronize_runtime_narrowing_views();
        }
        self
    }

    #[must_use]
    pub fn with_subagent_execution(
        mut self,
        subagent_execution: ConstrainedSubagentExecution,
    ) -> Self {
        let existing_contract = self.subagent_contract.take();
        let existing_identity = existing_contract
            .as_ref()
            .and_then(ConstrainedSubagentContractView::resolved_identity)
            .cloned();
        let existing_profile = existing_contract
            .as_ref()
            .and_then(|contract| contract.profile);
        let existing_runtime_narrowing = existing_contract
            .as_ref()
            .map(|contract| contract.runtime_narrowing.clone())
            .filter(|runtime_narrowing| !runtime_narrowing.is_empty());
        let mut subagent_execution = subagent_execution.with_resolved_profile();
        if subagent_execution.identity.is_none()
            && let Some(identity) = existing_identity
        {
            subagent_execution.identity = Some(identity);
        }
        let mut merged_contract = subagent_execution.contract_view();
        if merged_contract.profile.is_none()
            && let Some(profile) = existing_profile
        {
            merged_contract = merged_contract.with_profile(profile);
        }
        if merged_contract.runtime_narrowing.is_empty()
            && let Some(runtime_narrowing) = existing_runtime_narrowing
        {
            merged_contract = merged_contract.with_runtime_narrowing(runtime_narrowing);
        }
        self.subagent_contract = Some(merged_contract);
        self.subagent_execution = Some(subagent_execution);
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_profile(mut self, subagent_profile: ConstrainedSubagentProfile) -> Self {
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.profile = Some(subagent_profile);
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_profile(subagent_profile));
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_identity(
        mut self,
        subagent_identity: ConstrainedSubagentIdentity,
    ) -> Self {
        if subagent_identity.is_empty() {
            return self;
        }
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.identity = Some(subagent_identity.clone());
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_identity(subagent_identity));
        self.synchronize_runtime_narrowing_views();
        self
    }

    pub fn resolved_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolve_runtime_narrowing_ref()
    }

    pub fn resolved_subagent_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::resolved_profile)
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_profile)
            })
    }

    pub fn resolved_subagent_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.subagent_execution
            .as_ref()
            .and_then(|execution| execution.identity.as_ref())
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_identity)
            })
    }

    pub fn resolved_subagent_contract(&self) -> Option<ConstrainedSubagentContractView> {
        let mut contract = self
            .subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::contract_view)
            .or(self.subagent_contract.clone())?;
        if let Some(stored_contract) = self.subagent_contract.as_ref()
            && contract.profile.is_none()
            && let Some(profile) = stored_contract.profile
        {
            contract = contract.with_profile(profile);
        }
        let resolved_runtime_narrowing = self.resolved_runtime_narrowing().cloned();
        if let Some(runtime_narrowing) = resolved_runtime_narrowing {
            contract = contract.with_runtime_narrowing(runtime_narrowing);
        }
        (!contract.is_empty()).then_some(contract)
    }

    pub fn subagent_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolved_runtime_narrowing()
    }

    #[must_use]
    pub(crate) fn with_runtime_self_continuity(
        mut self,
        runtime_self_continuity: RuntimeSelfContinuity,
    ) -> Self {
        if !runtime_self_continuity.is_empty() {
            self.runtime_self_continuity = Some(runtime_self_continuity);
        }
        self
    }

    fn resolve_runtime_narrowing_owned(&self) -> Option<ToolRuntimeNarrowing> {
        let resolved_runtime_narrowing = self.resolve_runtime_narrowing_ref();
        resolved_runtime_narrowing.cloned()
    }

    fn synchronize_runtime_narrowing_views(&mut self) {
        let resolved_runtime_narrowing = self.resolve_runtime_narrowing_owned();
        let execution_runtime_narrowing = resolved_runtime_narrowing.clone().unwrap_or_default();
        let contract_runtime_narrowing = execution_runtime_narrowing.clone();

        self.runtime_narrowing = resolved_runtime_narrowing;
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.runtime_narrowing = execution_runtime_narrowing;
        }
        if let Some(subagent_contract) = self.subagent_contract.as_mut() {
            subagent_contract.runtime_narrowing = contract_runtime_narrowing;
        }
    }

    fn resolve_runtime_narrowing_ref(&self) -> Option<&ToolRuntimeNarrowing> {
        let session_runtime_narrowing =
            non_empty_runtime_narrowing_ref(self.runtime_narrowing.as_ref());
        if let Some(session_runtime_narrowing) = session_runtime_narrowing {
            return Some(session_runtime_narrowing);
        }

        let execution_runtime_narrowing_source = self
            .subagent_execution
            .as_ref()
            .map(|execution| &execution.runtime_narrowing);
        let execution_runtime_narrowing =
            non_empty_runtime_narrowing_ref(execution_runtime_narrowing_source);
        if let Some(execution_runtime_narrowing) = execution_runtime_narrowing {
            return Some(execution_runtime_narrowing);
        }

        let contract_runtime_narrowing_source = self
            .subagent_contract
            .as_ref()
            .map(|contract| &contract.runtime_narrowing);
        non_empty_runtime_narrowing_ref(contract_runtime_narrowing_source)
    }
}

fn non_empty_runtime_narrowing_ref(
    runtime_narrowing: Option<&ToolRuntimeNarrowing>,
) -> Option<&ToolRuntimeNarrowing> {
    runtime_narrowing.filter(|runtime_narrowing| !runtime_narrowing.is_empty())
}

fn normalize_session_id(session_id: String) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(feature = "memory-sqlite")]
fn load_session_tool_policy(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<SessionToolPolicyRecord>, String> {
    repo.load_session_tool_policy(session_id)
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_tool_policy_to_tool_view(
    base_tool_view: ToolView,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> ToolView {
    let Some(session_tool_policy) = session_tool_policy else {
        return base_tool_view;
    };
    if session_tool_policy.requested_tool_ids.is_empty() {
        return base_tool_view;
    }

    let policy_tool_view = ToolView::from_tool_names(session_tool_policy.requested_tool_ids.iter());
    base_tool_view.intersect(&policy_tool_view)
}

#[cfg(feature = "memory-sqlite")]
fn merge_effective_runtime_narrowing(
    delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> Option<ToolRuntimeNarrowing> {
    let policy_runtime_narrowing = session_tool_policy.and_then(|policy| {
        (!policy.runtime_narrowing.is_empty()).then_some(policy.runtime_narrowing.clone())
    });
    crate::tools::runtime_config::merge_runtime_narrowing_sources(
        delegate_runtime_narrowing,
        policy_runtime_narrowing,
    )
}

#[cfg(feature = "memory-sqlite")]
fn load_session_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<RuntimeSelfContinuity>, String> {
    runtime_self_continuity::load_persisted_runtime_self_continuity(repo, session_id)
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct PersistedSessionSnapshot {
    session_id: String,
    parent_session_id: Option<String>,
    label: Option<String>,
    is_delegate_child: bool,
    subagent_execution: Option<ConstrainedSubagentExecution>,
    session_tool_policy: Option<SessionToolPolicyRecord>,
    delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

#[cfg(feature = "memory-sqlite")]
fn open_session_repository(config: &LoongClawConfig) -> CliResult<SessionRepository> {
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    SessionRepository::new(&memory_config)
        .map_err(|error| format!("open session repository failed: {error}"))
}

#[cfg(feature = "memory-sqlite")]
fn load_persisted_session_snapshot(
    repo: &SessionRepository,
    session_id: &str,
) -> CliResult<Option<PersistedSessionSnapshot>> {
    let session_tool_policy = load_session_tool_policy(repo, session_id)?;
    let session = repo
        .load_session(session_id)
        .map_err(|error| format!("load session context failed: {error}"))?;

    if let Some(session) = session {
        let parent_session_id = session.parent_session_id;
        let is_delegate_child = parent_session_id.is_some();
        let label = session.label;
        let subagent_execution = if is_delegate_child {
            load_delegate_execution(repo, session_id)?
        } else {
            None
        };
        let delegate_runtime_narrowing = if is_delegate_child {
            subagent_execution.as_ref().and_then(|execution| {
                (!execution.runtime_narrowing.is_empty())
                    .then_some(execution.runtime_narrowing.clone())
            })
        } else {
            None
        };
        let runtime_self_continuity = load_session_runtime_self_continuity(repo, session_id)?;
        let snapshot = PersistedSessionSnapshot {
            session_id: session.session_id,
            parent_session_id,
            label,
            is_delegate_child,
            subagent_execution,
            session_tool_policy,
            delegate_runtime_narrowing,
            runtime_self_continuity,
        };
        return Ok(Some(snapshot));
    }

    let summary = repo
        .load_session_summary_with_legacy_fallback(session_id)
        .map_err(|error| format!("load legacy session context failed: {error}"))?;

    let Some(summary) = summary else {
        return Ok(None);
    };

    let is_delegate_child = summary.kind == SessionKind::DelegateChild;
    let subagent_execution = if is_delegate_child {
        load_delegate_execution(repo, session_id)?
    } else {
        None
    };
    let delegate_runtime_narrowing = if is_delegate_child {
        subagent_execution.as_ref().and_then(|execution| {
            (!execution.runtime_narrowing.is_empty()).then_some(execution.runtime_narrowing.clone())
        })
    } else {
        None
    };
    let runtime_self_continuity = load_session_runtime_self_continuity(repo, session_id)?;
    let snapshot = PersistedSessionSnapshot {
        session_id: summary.session_id,
        parent_session_id: summary.parent_session_id,
        label: summary.label,
        is_delegate_child,
        subagent_execution,
        session_tool_policy,
        delegate_runtime_narrowing,
        runtime_self_continuity,
    };
    Ok(Some(snapshot))
}

#[cfg(feature = "memory-sqlite")]
fn build_base_tool_view_from_snapshot(
    config: &LoongClawConfig,
    repo: &SessionRepository,
    session_id: &str,
    snapshot: Option<&PersistedSessionSnapshot>,
) -> CliResult<ToolView> {
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None);
    let Some(snapshot) = snapshot else {
        return Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
            config,
        ));
    };

    let is_delegate_child = snapshot.parent_session_id.is_some() || snapshot.is_delegate_child;
    if is_delegate_child {
        let derived_contract =
            resolve_delegate_child_contract(repo, session_id, config.tools.delegate.max_depth)?;
        return Ok(delegate_child_tool_view_for_runtime_config_and_contract(
            &config.tools,
            &tool_runtime_config,
            derived_contract.as_ref(),
        ));
    }

    Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
        config,
    ))
}

#[derive(Clone)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub label: Option<String>,
    pub execution: ConstrainedSubagentExecution,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
    pub timeout_seconds: u64,
    pub binding: OwnedConversationRuntimeBinding,
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct DefaultAsyncDelegateSpawner {
    config: Arc<LoongClawConfig>,
}

#[cfg(feature = "memory-sqlite")]
impl DefaultAsyncDelegateSpawner {
    fn new(config: &LoongClawConfig) -> Self {
        Self {
            config: Arc::new(config.clone()),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for DefaultAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        let AsyncDelegateSpawnRequest {
            child_session_id,
            parent_session_id,
            task,
            label,
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

        let repo = SessionRepository::new(&MemoryRuntimeConfig::from_memory_config(
            &self.config.memory,
        ))?;
        let runtime = DefaultConversationRuntime::from_config_or_env(self.config.as_ref())?;
        let runtime_ref = &runtime;
        let child_session_id_for_spawn = child_session_id.clone();
        let parent_session_id_for_spawn = parent_session_id.clone();
        let borrowed_binding = binding.as_borrowed();
        let child_binding = binding.clone();
        super::turn_coordinator::with_prepared_subagent_spawn_cleanup_if_kernel_bound(
            runtime_ref,
            &parent_session_id,
            &child_session_id,
            borrowed_binding,
            move || async move {
                let started = repo.transition_session_with_event_if_current(
                    &child_session_id_for_spawn,
                    TransitionSessionWithEventIfCurrentRequest {
                        expected_state: SessionState::Ready,
                        next_state: SessionState::Running,
                        last_error: None,
                        event_kind: "delegate_started".to_owned(),
                        actor_session_id: Some(parent_session_id_for_spawn.clone()),
                        event_payload_json: execution.spawn_payload_with_runtime_self_continuity(
                            &task,
                            label.as_deref(),
                            runtime_self_continuity.as_ref(),
                        ),
                    },
                )?;
                if started.is_none() {
                    return Err(format!(
                        "async_delegate_spawn_skipped: session `{}` was not in Ready state",
                        child_session_id_for_spawn
                    ));
                }

                let _ = super::turn_coordinator::run_started_delegate_child_turn_with_runtime(
                    self.config.as_ref(),
                    runtime_ref,
                    &child_session_id_for_spawn,
                    &parent_session_id_for_spawn,
                    label,
                    &task,
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
}

pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
    turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEngineSelectionSource {
    Env,
    Config,
    Default,
}

impl ContextEngineSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextEngineSelectionSource::Env => "env",
            ContextEngineSelectionSource::Config => "config",
            ContextEngineSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMiddlewareSelectionSource {
    Env,
    Config,
    Default,
}

impl TurnMiddlewareSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnMiddlewareSelectionSource::Env => "env",
            TurnMiddlewareSelectionSource::Config => "config",
            TurnMiddlewareSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineSelection {
    pub id: String,
    pub source: ContextEngineSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareSelection {
    pub ids: Vec<String>,
    pub source: TurnMiddlewareSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionPolicySnapshot {
    pub enabled: bool,
    pub min_messages: Option<usize>,
    pub trigger_estimated_tokens: Option<usize>,
    pub fail_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineRuntimeSnapshot {
    pub selected: ContextEngineSelection,
    pub selected_metadata: ContextEngineMetadata,
    pub available: Vec<ContextEngineMetadata>,
    pub turn_middlewares: TurnMiddlewareRuntimeSnapshot,
    pub compaction: ContextCompactionPolicySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareRuntimeSnapshot {
    pub selected: TurnMiddlewareSelection,
    pub selected_metadata: Vec<TurnMiddlewareMetadata>,
    pub available: Vec<TurnMiddlewareMetadata>,
}

pub fn resolve_context_engine_selection(config: &LoongClawConfig) -> ContextEngineSelection {
    if let Some(id) = context_engine_id_from_env() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Env,
        };
    }

    if let Some(id) = config.conversation.context_engine_id() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Config,
        };
    }

    ContextEngineSelection {
        id: DEFAULT_CONTEXT_ENGINE_ID.to_owned(),
        source: ContextEngineSelectionSource::Default,
    }
}

pub fn resolve_turn_middleware_selection(
    config: &LoongClawConfig,
) -> CliResult<TurnMiddlewareSelection> {
    let mut ids = default_turn_middleware_ids()?;
    if let Some(env_ids) = turn_middleware_ids_from_env() {
        ids.extend(env_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Env,
        });
    }

    let configured_ids = config.conversation.turn_middleware_ids();
    if !configured_ids.is_empty() {
        ids.extend(configured_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Config,
        });
    }

    Ok(TurnMiddlewareSelection {
        ids: normalize_turn_middleware_ids(ids),
        source: TurnMiddlewareSelectionSource::Default,
    })
}

pub fn collect_context_engine_runtime_snapshot(
    config: &LoongClawConfig,
) -> CliResult<ContextEngineRuntimeSnapshot> {
    let selected = resolve_context_engine_selection(config);
    let selected_metadata = describe_context_engine(Some(selected.id.as_str()))?;
    let available = list_context_engine_metadata()?;
    let turn_middleware_selection = resolve_turn_middleware_selection(config)?;
    let turn_middlewares = TurnMiddlewareRuntimeSnapshot {
        selected_metadata: describe_turn_middlewares(turn_middleware_selection.ids.as_slice())?,
        available: list_turn_middleware_metadata()?,
        selected: turn_middleware_selection,
    };
    let compaction = ContextCompactionPolicySnapshot {
        enabled: config.conversation.compact_enabled,
        min_messages: config.conversation.compact_min_messages(),
        trigger_estimated_tokens: config.conversation.compact_trigger_estimated_tokens(),
        fail_open: config.conversation.compaction_fail_open(),
    };

    Ok(ContextEngineRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        turn_middlewares,
        compaction,
    })
}

impl Default for DefaultConversationRuntime<DefaultContextEngine> {
    fn default() -> Self {
        Self::with_context_engine(DefaultContextEngine)
    }
}

impl DefaultConversationRuntime<DefaultContextEngine> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_turn_middlewares(
        turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
    ) -> Self {
        Self::with_context_engine_and_turn_middlewares(DefaultContextEngine, turn_middlewares)
    }
}

impl<E> DefaultConversationRuntime<E> {
    pub fn with_context_engine(context_engine: E) -> Self {
        Self {
            context_engine,
            turn_middlewares: builtin_turn_middlewares(),
        }
    }

    pub fn with_context_engine_and_turn_middlewares(
        context_engine: E,
        turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
    ) -> Self {
        let mut combined_turn_middlewares = builtin_turn_middlewares();
        combined_turn_middlewares.extend(turn_middlewares);
        Self {
            context_engine,
            turn_middlewares: combined_turn_middlewares,
        }
    }
}

impl<E> DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    async fn build_context_for_tool_view(
        &self,
        config: &LoongClawConfig,
        session_context: &SessionContext,
        include_system_prompt: bool,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let runtime_tool_view = crate::tools::runtime_tool_view_from_loongclaw_config(config);
        let mut assembled = self
            .context_engine
            .assemble_context(
                config,
                session_context.session_id.as_str(),
                include_system_prompt,
                binding,
            )
            .await?;
        let runtime_self_continuity = include_system_prompt
            .then(|| runtime_self_continuity_prompt_summary(config, session_context))
            .flatten();
        let delegate_runtime_contract = include_system_prompt
            .then(|| delegate_child_runtime_contract_prompt_summary(config, session_context))
            .flatten();

        seed_prompt_fragments_from_context(&mut assembled);
        append_runtime_prompt_fragment(
            &mut assembled,
            "runtime-self-continuity",
            runtime_self_continuity,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-runtime-contract",
            delegate_runtime_contract,
        );
        sync_prompt_fragments_into_context(&mut assembled);

        self.apply_turn_middlewares_to_context(
            config,
            session_context.session_id.as_str(),
            include_system_prompt,
            assembled,
            &runtime_tool_view,
            requested_tool_view,
            binding,
        )
        .await
    }

    pub fn context_engine_metadata(&self) -> ContextEngineMetadata {
        self.context_engine.metadata()
    }

    pub fn turn_middleware_metadata(&self) -> Vec<TurnMiddlewareMetadata> {
        self.turn_middlewares
            .iter()
            .map(|middleware| middleware.metadata())
            .collect()
    }

    async fn run_turn_middlewares_bootstrap(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.bootstrap(config, session_id, kernel_ctx).await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.ingest(session_id, message, kernel_ctx).await?;
        }
        Ok(())
    }

    async fn apply_turn_middlewares_to_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        for middleware in &self.turn_middlewares {
            assembled = middleware
                .transform_context(
                    config,
                    session_id,
                    include_system_prompt,
                    assembled,
                    runtime_tool_view,
                    requested_tool_view,
                    binding,
                )
                .await?;
        }
        Ok(assembled)
    }

    async fn run_turn_middlewares_after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .after_turn(
                    session_id,
                    user_input,
                    assistant_reply,
                    messages,
                    kernel_ctx,
                )
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_compact_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .compact_context(config, session_id, messages, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }
}

impl DefaultConversationRuntime<Box<dyn ConversationContextEngine>> {
    pub fn from_engine_id(engine_id: Option<&str>) -> CliResult<Self> {
        let context_engine = resolve_context_engine(engine_id)?;
        Ok(Self::with_context_engine(context_engine))
    }

    pub fn from_config_or_env(config: &LoongClawConfig) -> CliResult<Self> {
        let selection = resolve_context_engine_selection(config);
        let turn_middleware_selection = resolve_turn_middleware_selection(config)?;
        let context_engine = resolve_context_engine(Some(selection.id.as_str()))?;
        let turn_middlewares = resolve_turn_middlewares(turn_middleware_selection.ids.as_slice())?;
        Ok(Self {
            context_engine,
            turn_middlewares,
        })
    }
}

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn session_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        Ok(SessionContext::root_with_tool_view(
            session_id,
            self.tool_view(config, session_id, binding)?,
        ))
    }

    fn tool_view(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        let _ = (session_id, binding);
        Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
            config,
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        config: &LoongClawConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        Some(Arc::new(DefaultAsyncDelegateSpawner::new(config)))
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn build_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_messages(
            config,
            session_id,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
        .await
        .map(AssembledConversationContext::from_messages)
    }
    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String>;

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_streaming(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn>;

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()>;

    async fn after_turn(
        &self,
        _session_id: &str,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[async_trait]
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    fn session_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;
            let session_tool_policy = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_tool_policy.as_ref());
            let tool_view =
                apply_session_tool_policy_to_tool_view(base_tool_view, session_tool_policy);

            if let Some(snapshot) = snapshot {
                let runtime_narrowing = merge_effective_runtime_narrowing(
                    snapshot.delegate_runtime_narrowing.clone(),
                    snapshot.session_tool_policy.as_ref(),
                );
                let mut session_context = match snapshot.parent_session_id {
                    Some(parent_session_id) => {
                        SessionContext::child(snapshot.session_id, parent_session_id, tool_view)
                    }
                    None => SessionContext::root_with_tool_view(snapshot.session_id, tool_view),
                };
                if snapshot.is_delegate_child {
                    if let Some(label) = snapshot.label {
                        session_context =
                            session_context.with_subagent_identity(ConstrainedSubagentIdentity {
                                nickname: Some(label),
                                specialization: None,
                            });
                    }
                    if let Some(subagent_execution) = snapshot.subagent_execution {
                        session_context =
                            session_context.with_subagent_execution(subagent_execution);
                    } else if let Some(subagent_profile) = derive_subagent_profile_from_lineage(
                        &repo,
                        session_id,
                        config.tools.delegate.max_depth,
                    )? {
                        session_context = session_context.with_subagent_profile(subagent_profile);
                    }
                }
                if let Some(runtime_narrowing) = runtime_narrowing {
                    session_context = session_context.with_runtime_narrowing(runtime_narrowing);
                }
                if let Some(runtime_self_continuity) = snapshot.runtime_self_continuity {
                    session_context =
                        session_context.with_runtime_self_continuity(runtime_self_continuity);
                }
                return Ok(session_context);
            }

            Ok(SessionContext::root_with_tool_view(session_id, tool_view))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        let tool_view = self.tool_view(config, session_id, _binding)?;

        #[cfg(not(feature = "memory-sqlite"))]
        Ok(SessionContext::root_with_tool_view(session_id, tool_view))
    }

    fn tool_view(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;
            let session_tool_policy = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.session_tool_policy.as_ref());
            Ok(apply_session_tool_policy_to_tool_view(
                base_tool_view,
                session_tool_policy,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        Ok(crate::tools::runtime_tool_view_from_loongclaw_config(
            config,
        ))
    }

    async fn bootstrap(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        let result = self
            .context_engine
            .bootstrap(config, session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_bootstrap(config, session_id, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        let result = self
            .context_engine
            .ingest(session_id, message, kernel_ctx)
            .await?;
        self.run_turn_middlewares_ingest(session_id, message, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn build_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
        .await
    }

    async fn build_messages(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            tool_view,
            binding,
        )
        .await
        .map(|assembled| assembled.messages)
    }

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, provider_runtime_binding(binding)).await
    }

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
        )
        .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
        )
        .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        if let Some(ctx) = binding.kernel_context() {
            let request = memory::build_append_turn_request(session_id, role, content);
            let caps = BTreeSet::from([Capability::MemoryWrite]);
            ctx.kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("persist {role} turn via kernel failed: {error}"))?;
            return Ok(());
        }

        #[cfg(feature = "memory-sqlite")]
        {
            memory::append_turn_direct(
                session_id,
                role,
                content,
                memory::runtime_config::get_memory_runtime_config(),
            )
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_id, role, content);
        }

        Ok(())
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await?;
        self.run_turn_middlewares_after_turn(
            session_id,
            user_input,
            assistant_reply,
            messages,
            kernel_ctx,
        )
        .await
    }

    async fn compact_context(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .compact_context(config, session_id, messages, kernel_ctx)
            .await?;
        self.run_turn_middlewares_compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_prepare_subagent_spawn(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_on_subagent_ended(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }
}

fn provider_runtime_binding(
    binding: ConversationRuntimeBinding<'_>,
) -> provider::ProviderRuntimeBinding<'_> {
    match binding {
        ConversationRuntimeBinding::Kernel(kernel_ctx) => {
            provider::ProviderRuntimeBinding::kernel(kernel_ctx)
        }
        ConversationRuntimeBinding::Direct => provider::ProviderRuntimeBinding::advisory_only(),
    }
}

fn delegate_child_runtime_contract_prompt_summary(
    config: &LoongClawConfig,
    session_context: &SessionContext,
) -> Option<String> {
    session_context.parent_session_id.as_ref()?;
    session_context.subagent_runtime_narrowing()?;
    let subagent_contract = session_context.resolved_subagent_contract();
    crate::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None)
        .delegate_child_prompt_summary(subagent_contract.as_ref())
}

fn runtime_self_continuity_prompt_summary(
    config: &LoongClawConfig,
    session_context: &SessionContext,
) -> Option<String> {
    let stored_continuity = session_context.runtime_self_continuity.as_ref()?;
    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let missing_continuity = runtime_self_continuity::missing_runtime_self_continuity(
        stored_continuity,
        live_continuity.as_ref(),
    )?;
    let inherited = session_context.parent_session_id.is_some();
    runtime_self_continuity::render_runtime_self_continuity_section(&missing_continuity, inherited)
}

fn append_runtime_prompt_fragment(
    assembled: &mut AssembledConversationContext,
    source_id: &'static str,
    content: Option<String>,
) {
    let Some(content) = content else {
        return;
    };

    let fragment = PromptFragment::new(
        source_id,
        PromptLane::Continuity,
        source_id,
        content,
        ContextArtifactKind::RuntimeContract,
    )
    .with_dedupe_key(source_id);

    assembled.prompt_fragments.push(fragment);
}

fn normalize_turn_middleware_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            normalized.push(id);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TurnTestHarness;

    #[test]
    fn provider_runtime_binding_maps_direct_conversation_binding_to_advisory_only() {
        assert!(matches!(
            provider_runtime_binding(ConversationRuntimeBinding::direct()),
            provider::ProviderRuntimeBinding::AdvisoryOnly
        ));
    }

    #[test]
    fn provider_runtime_binding_maps_kernel_conversation_binding_to_kernel() {
        let harness = TurnTestHarness::new();

        assert!(matches!(
            provider_runtime_binding(ConversationRuntimeBinding::kernel(&harness.kernel_ctx)),
            provider::ProviderRuntimeBinding::Kernel(kernel_ctx)
                if std::ptr::eq(kernel_ctx, &harness.kernel_ctx)
        ));
    }
}
