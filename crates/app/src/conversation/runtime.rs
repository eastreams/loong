use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::Capability;
use serde_json::Value;

use crate::CliResult;
use crate::KernelContext;
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::ToolView;

use super::super::memory;
use super::super::{config::LoongConfig, provider};
#[cfg(feature = "memory-sqlite")]
use super::active_external_skills;
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
#[cfg(test)]
use super::context_engine::ContextArtifactKind;
#[cfg(test)]
use super::subagent::DelegateBuiltinProfile;
use super::context_engine_registry::{
    DEFAULT_CONTEXT_ENGINE_ID, context_engine_id_from_env, describe_context_engine,
    list_context_engine_metadata, resolve_context_engine,
};
use super::mailbox_for_session;
use super::prompt_orchestrator::seed_prompt_fragments_from_context;
use super::prompt_orchestrator::sync_prompt_fragments_into_context;
use super::runtime_binding::ConversationRuntimeBinding;
#[cfg(test)]
use super::runtime_binding::OwnedConversationRuntimeBinding;
use super::turn_engine::ProviderTurn;
use super::turn_middleware::{
    ConversationTurnMiddleware, TurnMiddlewareMetadata, builtin_turn_middlewares,
};
use super::turn_middleware_registry::{
    default_turn_middleware_ids, describe_turn_middlewares, list_turn_middleware_metadata,
    resolve_turn_middlewares, turn_middleware_ids_from_env,
};
use super::{PromptFragment, PromptFrameAuthority, PromptLane};

#[path = "runtime_session.rs"]
mod session_runtime;
#[path = "runtime_context.rs"]
mod runtime_context;
#[path = "runtime_prompt.rs"]
mod runtime_prompt;
#[path = "runtime_delegate.rs"]
mod runtime_delegate;
#[path = "runtime_turn_middleware.rs"]
mod runtime_turn_middleware;
#[cfg(feature = "memory-sqlite")]
use session_runtime::{
    apply_active_external_skill_blocked_tools_to_tool_view, apply_session_tool_policy_to_tool_view,
    build_base_tool_view_from_snapshot, build_session_context_from_snapshot,
    load_persisted_session_context, load_persisted_session_snapshot, open_session_repository,
};
pub use runtime_context::SessionContext;
use runtime_context::{
    model_visible_external_skill_roots_from_config, root_session_context_from_config,
};
use runtime_prompt::{
    active_external_skills_prompt_summary, append_runtime_prompt_fragment,
    delegate_child_profile_prompt_summary, delegate_child_runtime_contract_prompt_summary,
    normalize_turn_middleware_ids, provider_runtime_binding,
    runtime_self_continuity_prompt_summary,
};
pub use runtime_delegate::{
    AsyncDelegateSpawnRequest, AsyncDelegateSpawner,
    async_delegate_spawn_request_from_serialized_parts,
};
#[cfg(feature = "memory-sqlite")]
pub use runtime_delegate::execute_async_delegate_spawn_request;
#[cfg(feature = "memory-sqlite")]
use runtime_delegate::DefaultAsyncDelegateSpawner;

#[cfg(feature = "memory-sqlite")]
use crate::session::store;
#[cfg(test)]
use crate::tools::runtime_config::ToolRuntimeNarrowing;




pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
    turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
}

pub type BoxedDefaultConversationRuntime =
    DefaultConversationRuntime<Box<dyn ConversationContextEngine>>;

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub struct HostedConversationRuntime<R> {
    inner: R,
    memory_config: store::SessionStoreConfig,
    async_delegate_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
    background_task_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
}

#[cfg(feature = "memory-sqlite")]
impl<R> HostedConversationRuntime<R> {
    pub fn new(inner: R) -> Self {
        let memory_config = store::current_session_store_config().clone();
        Self::new_with_memory_config(inner, memory_config)
    }

    pub fn new_with_memory_config(inner: R, memory_config: store::SessionStoreConfig) -> Self {
        Self {
            inner,
            memory_config,
            async_delegate_spawner_override: None,
            background_task_spawner_override: None,
        }
    }

    #[must_use]
    pub fn with_async_delegate_spawner(
        mut self,
        async_delegate_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.async_delegate_spawner_override = Some(async_delegate_spawner);
        self
    }

    #[must_use]
    pub fn with_background_task_spawner(
        mut self,
        background_task_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.background_task_spawner_override = Some(background_task_spawner);
        self
    }
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

pub fn resolve_context_engine_selection(config: &LoongConfig) -> ContextEngineSelection {
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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
        config: &LoongConfig,
        session_context: &SessionContext,
        include_system_prompt: bool,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let effective_config_storage;
        let effective_config = match session_context.workspace_root.as_ref() {
            Some(workspace_root) => {
                let mut overridden_config = config.clone();
                overridden_config.tools.file_root = Some(workspace_root.display().to_string());
                effective_config_storage = overridden_config;
                &effective_config_storage
            }
            None => config,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view_from_loong_config(effective_config);
        let mut assembled = self
            .context_engine
            .assemble_context(
                effective_config,
                session_context.session_id.as_str(),
                include_system_prompt,
                binding,
            )
            .await?;
        let runtime_self_continuity = include_system_prompt
            .then(|| runtime_self_continuity_prompt_summary(effective_config, session_context))
            .flatten();
        #[cfg(feature = "memory-sqlite")]
        let active_external_skills = include_system_prompt
            .then(|| {
                active_external_skills_prompt_summary(
                    effective_config,
                    session_context.session_id.as_str(),
                )
            })
            .flatten();
        #[cfg(not(feature = "memory-sqlite"))]
        let active_external_skills: Option<String> = None;
        let delegate_runtime_contract = include_system_prompt
            .then(|| {
                delegate_child_runtime_contract_prompt_summary(effective_config, session_context)
            })
            .flatten();
        let delegate_profile_contract = include_system_prompt
            .then(|| delegate_child_profile_prompt_summary(session_context))
            .flatten();

        seed_prompt_fragments_from_context(&mut assembled);
        append_runtime_prompt_fragment(
            &mut assembled,
            "runtime-self-continuity",
            runtime_self_continuity,
            PromptFrameAuthority::RuntimeSelf,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "active-external-skills",
            active_external_skills,
            PromptFrameAuthority::SessionLocalRecall,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-profile",
            delegate_profile_contract,
            PromptFrameAuthority::AdvisoryProfile,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-runtime-contract",
            delegate_runtime_contract,
            PromptFrameAuthority::CapabilityContract,
        );
        sync_prompt_fragments_into_context(&mut assembled);

        self.apply_turn_middlewares_to_context(
            effective_config,
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
}

impl DefaultConversationRuntime<Box<dyn ConversationContextEngine>> {
    pub fn from_engine_id(engine_id: Option<&str>) -> CliResult<Self> {
        let context_engine = resolve_context_engine(engine_id)?;
        Ok(Self::with_context_engine(context_engine))
    }

    pub fn from_config_or_env(config: &LoongConfig) -> CliResult<Self> {
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

pub fn load_default_conversation_runtime(
    config: &LoongConfig,
) -> CliResult<BoxedDefaultConversationRuntime> {
    BoxedDefaultConversationRuntime::from_config_or_env(config)
}

#[cfg(feature = "memory-sqlite")]
pub fn load_hosted_default_conversation_runtime(
    config: &LoongConfig,
) -> CliResult<HostedConversationRuntime<BoxedDefaultConversationRuntime>> {
    let inner_runtime = load_default_conversation_runtime(config)?;
    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    let runtime = HostedConversationRuntime::new_with_memory_config(inner_runtime, memory_config);
    Ok(runtime)
}

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        let tool_view = self.tool_view(config, session_id, binding)?;

        #[cfg(feature = "memory-sqlite")]
        if let Some(session_context) =
            load_persisted_session_context(config, session_id, &tool_view)?
        {
            return Ok(session_context);
        }

        Ok(root_session_context_from_config(
            config, session_id, tool_view,
        ))
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        let _ = (session_id, binding);
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        Some(Arc::new(DefaultAsyncDelegateSpawner::new(config)))
    }

    #[cfg(feature = "memory-sqlite")]
    fn background_task_spawner(
        &self,
        _config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        None
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
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
        config: &LoongConfig,
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
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String>;

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.request_completion(config, messages, binding).await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn_streaming(
            config, session_id, turn_id, messages, tool_view, binding, on_token,
        )
        .await
    }

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
        _config: &LoongConfig,
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

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> ConversationRuntime for HostedConversationRuntime<R>
where
    R: ConversationRuntime,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        self.inner.session_context(config, session_id, binding)
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        self.inner.tool_view(config, session_id, binding)
    }

    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.async_delegate_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.async_delegate_spawner(config),
        }
    }

    fn background_task_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.background_task_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.background_task_spawner(config),
        }
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.inner.bootstrap(config, session_id, kernel_ctx).await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        self.inner.ingest(session_id, message, kernel_ctx).await
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.inner
            .build_context(config, session_id, include_system_prompt, binding)
            .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.inner
            .build_messages(
                config,
                session_id,
                include_system_prompt,
                tool_view,
                binding,
            )
            .await
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.inner
            .request_completion(config, messages, binding)
            .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
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
        if binding.kernel_context().is_some() {
            return self
                .inner
                .persist_turn(session_id, role, content, binding)
                .await;
        }

        store::append_session_turn_direct(session_id, role, content, &self.memory_config)
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;

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
        self.inner
            .after_turn(
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
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}

#[async_trait]
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;

            if let Some(snapshot) = snapshot {
                return build_session_context_from_snapshot(
                    config,
                    &repo,
                    session_id,
                    base_tool_view,
                    snapshot,
                );
            }

            Ok(root_session_context_from_config(
                config,
                session_id,
                base_tool_view,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let tool_view = self.tool_view(config, session_id, _binding)?;
            Ok(root_session_context_from_config(
                config, session_id, tool_view,
            ))
        }
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;
            let tool_view = apply_session_tool_policy_to_tool_view(
                base_tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.session_tool_policy.as_ref()),
            );
            Ok(apply_active_external_skill_blocked_tools_to_tool_view(
                tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active_external_skills.as_ref()),
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
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
        config: &LoongConfig,
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
        config: &LoongConfig,
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
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, provider_runtime_binding(binding)).await
    }

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        provider::request_completion_with_retry_progress(
            config,
            messages,
            provider_runtime_binding(binding),
            retry_progress,
        )
        .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
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

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            retry_progress,
        )
        .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
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

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
            retry_progress,
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
            store::append_session_turn_direct(
                session_id,
                role,
                content,
                store::current_session_store_config(),
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
        config: &LoongConfig,
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


#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "memory-sqlite")]
    use crate::conversation::active_external_skills::{
        ACTIVE_EXTERNAL_SKILLS_EVENT_KIND, ActiveExternalSkill, ActiveExternalSkillsState,
    };
    #[cfg(feature = "memory-sqlite")]
    use crate::session::repository::{
        NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
    };
    use crate::test_support::TurnTestHarness;
    use crate::test_support::unique_temp_dir;
    #[cfg(feature = "memory-sqlite")]
    use serde_json::json;
    #[cfg(feature = "memory-sqlite")]
    use std::sync::Arc;

    #[cfg(feature = "memory-sqlite")]
    #[derive(Clone)]
    struct NoopTestSpawner;

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl AsyncDelegateSpawner for NoopTestSpawner {
        async fn spawn(&self, _request: AsyncDelegateSpawnRequest) -> Result<(), String> {
            Ok(())
        }
    }

    #[cfg(feature = "memory-sqlite")]
    struct SpawnerAwareRuntime {
        async_delegate_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
        background_task_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for SpawnerAwareRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        fn async_delegate_spawner(
            &self,
            _config: &LoongConfig,
        ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
            self.async_delegate_spawner.clone()
        }

        fn background_task_spawner(
            &self,
            _config: &LoongConfig,
        ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
            self.background_task_spawner.clone()
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

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

    #[test]
    fn normalize_turn_middleware_ids_preserves_first_occurrence_order() {
        let normalized = normalize_turn_middleware_ids(vec![
            "alpha".to_owned(),
            "beta".to_owned(),
            "alpha".to_owned(),
            "gamma".to_owned(),
            "beta".to_owned(),
        ]);

        assert_eq!(normalized, vec!["alpha", "beta", "gamma"]);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hosted_runtime_overrides_background_task_spawner_without_changing_async_delegate_spawner() {
        let config = LoongConfig::default();
        let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let override_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_runtime = SpawnerAwareRuntime {
            async_delegate_spawner: Some(inner_async_spawner.clone()),
            background_task_spawner: Some(inner_background_spawner),
        };

        let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
            .with_background_task_spawner(override_background_spawner.clone());

        let resolved_async_spawner = hosted_runtime
            .async_delegate_spawner(&config)
            .expect("async delegate spawner");
        let resolved_background_spawner = hosted_runtime
            .background_task_spawner(&config)
            .expect("background task spawner");

        assert!(Arc::ptr_eq(&resolved_async_spawner, &inner_async_spawner));
        assert!(Arc::ptr_eq(
            &resolved_background_spawner,
            &override_background_spawner
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hosted_runtime_overrides_async_delegate_spawner_without_changing_background_task_spawner() {
        let config = LoongConfig::default();
        let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let override_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_runtime = SpawnerAwareRuntime {
            async_delegate_spawner: Some(inner_async_spawner),
            background_task_spawner: Some(inner_background_spawner.clone()),
        };

        let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
            .with_async_delegate_spawner(override_async_spawner.clone());

        let resolved_async_spawner = hosted_runtime
            .async_delegate_spawner(&config)
            .expect("async delegate spawner");
        let resolved_background_spawner = hosted_runtime
            .background_task_spawner(&config)
            .expect("background task spawner");

        assert!(Arc::ptr_eq(
            &resolved_async_spawner,
            &override_async_spawner
        ));
        assert!(Arc::ptr_eq(
            &resolved_background_spawner,
            &inner_background_spawner
        ));
    }

    #[test]
    fn async_delegate_spawn_request_round_trips_runtime_self_continuity_json() {
        let execution = super::super::subagent::ConstrainedSubagentExecution {
            mode: super::super::subagent::ConstrainedSubagentMode::Async,
            isolation: super::super::subagent::ConstrainedSubagentIsolation::Shared,
            owner_kind: None,
            depth: 1,
            max_depth: 2,
            active_children: 0,
            max_active_children: 2,
            timeout_seconds: 30,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["file.read".to_owned()],
            workspace_root: None,
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            kernel_bound: false,
            identity: None,
            profile: Some(
                super::super::subagent::ConstrainedSubagentProfile::for_child_depth(1, 2),
            ),
        };
        let continuity = RuntimeSelfContinuity {
            workspace_guidance: crate::workspace_guidance::WorkspaceGuidanceModel {
                entries: vec!["Keep continuity explicit.".to_owned()],
            },
            runtime_self: crate::runtime_self::RuntimeSelfModel::default(),
            resolved_identity: None,
            session_profile_projection: Some("delegate profile".to_owned()),
        };
        let request = AsyncDelegateSpawnRequest {
            child_session_id: "child-1".to_owned(),
            parent_session_id: "parent-1".to_owned(),
            task: "investigate".to_owned(),
            canonical_task_id: Some("task-1".to_owned()),
            label: Some("child".to_owned()),
            profile: Some(DelegateBuiltinProfile::Research),
            execution: execution.clone(),
            runtime_self_continuity: Some(continuity.clone()),
            timeout_seconds: 30,
            binding: OwnedConversationRuntimeBinding::direct(),
        };

        let encoded = request
            .runtime_self_continuity_json()
            .expect("serialize runtime self continuity");
        let round_tripped = async_delegate_spawn_request_from_serialized_parts(
            request.child_session_id.clone(),
            request.parent_session_id.clone(),
            request.task.clone(),
            request.canonical_task_id.clone(),
            request.label.clone(),
            request.profile,
            execution,
            encoded,
            request.timeout_seconds,
            OwnedConversationRuntimeBinding::direct(),
        )
        .expect("round-trip async delegate request");

        assert_eq!(
            round_tripped.runtime_self_continuity,
            Some(continuity),
            "runtime self continuity should survive serialization"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn hosted_runtime_build_context_delegates_to_inner_runtime() {
        #[derive(Clone)]
        struct BuildContextAwareRuntime;

        #[async_trait]
        impl ConversationRuntime for BuildContextAwareRuntime {
            fn tool_view(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<ToolView> {
                Ok(crate::tools::runtime_tool_view())
            }

            async fn build_context(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _include_system_prompt: bool,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<AssembledConversationContext> {
                let messages = vec![serde_json::json!({
                    "role": "system",
                    "content": "delegated"
                })];
                let prompt_fragment = PromptFragment::new(
                    "fragment",
                    PromptLane::RuntimeSelf,
                    "runtime-self",
                    "delegated fragment",
                    ContextArtifactKind::RuntimeContract,
                );
                let assembled = AssembledConversationContext {
                    messages,
                    artifacts: Vec::new(),
                    estimated_tokens: Some(7),
                    prompt_fragments: vec![prompt_fragment],
                    system_prompt_addition: Some("addition".to_owned()),
                };

                Ok(assembled)
            }

            async fn build_messages(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _include_system_prompt: bool,
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<Vec<Value>> {
                Err("build_messages should not be used when build_context is delegated".to_owned())
            }

            async fn request_completion(
                &self,
                _config: &LoongConfig,
                _messages: &[Value],
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<String> {
                Ok(String::new())
            }

            async fn request_turn(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _turn_id: &str,
                _messages: &[Value],
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<ProviderTurn> {
                Err("unused".to_owned())
            }

            async fn request_turn_streaming(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _turn_id: &str,
                _messages: &[Value],
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
                _on_token: crate::provider::StreamingTokenCallback,
            ) -> CliResult<ProviderTurn> {
                Err("unused".to_owned())
            }

            async fn persist_turn(
                &self,
                _session_id: &str,
                _role: &str,
                _content: &str,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<()> {
                Ok(())
            }
        }

        let config = LoongConfig::default();
        let hosted_runtime = HostedConversationRuntime::new(BuildContextAwareRuntime);

        let assembled = hosted_runtime
            .build_context(
                &config,
                "session-1",
                true,
                ConversationRuntimeBinding::Direct,
            )
            .await
            .expect("delegated build_context");

        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.estimated_tokens, Some(7));
        assert_eq!(assembled.prompt_fragments.len(), 1);
        assert_eq!(
            assembled.system_prompt_addition.as_deref(),
            Some("addition")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn load_hosted_default_conversation_runtime_keeps_default_async_spawner_only() {
        let config = LoongConfig::default();
        let runtime = load_hosted_default_conversation_runtime(&config)
            .expect("load hosted default conversation runtime");

        let async_delegate_spawner = runtime.async_delegate_spawner(&config);
        let background_task_spawner = runtime.background_task_spawner(&config);

        assert!(async_delegate_spawner.is_some());
        assert!(background_task_spawner.is_none());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn hosted_runtime_persist_turn_uses_explicit_memory_config_for_direct_binding() {
        let root = unique_temp_dir("hosted-runtime-explicit-memory");
        let sqlite_path = root.join("explicit-memory.db");
        let memory_config =
            crate::session::store::SessionStoreConfig::for_sqlite_path(sqlite_path.clone());
        let session_id = "hosted-runtime-explicit-session";
        let runtime = HostedConversationRuntime::new_with_memory_config(
            SpawnerAwareRuntime {
                async_delegate_spawner: None,
                background_task_spawner: None,
            },
            memory_config.clone(),
        );

        crate::session::store::ensure_session_store_ready(Some(sqlite_path), &memory_config)
            .expect("initialize explicit session store");

        runtime
            .persist_turn(
                session_id,
                "assistant",
                "persist via explicit memory config",
                ConversationRuntimeBinding::Direct,
            )
            .await
            .expect("persist hosted runtime turn");

        let turns = crate::session::store::window_session_turns(session_id, 8, &memory_config)
            .expect("load persisted turns");
        let persisted_turn = turns
            .iter()
            .find(|turn| {
                turn.role == "assistant" && turn.content == "persist via explicit memory config"
            })
            .expect("persisted assistant turn");

        assert_eq!(persisted_turn.role, "assistant");
        assert_eq!(persisted_turn.content, "persist via explicit memory config");
    }

    #[tokio::test]
    async fn default_runtime_build_context_rehydrates_active_external_skills() {
        let runtime = DefaultConversationRuntime::default();
        let session_id = "session-active-external-skills";
        let root = unique_temp_dir("active-external-skills-runtime");
        let sqlite_path = root.join("memory.db");
        let workspace_root = root.join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");

        let mut config = LoongConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.tools.file_root = Some(workspace_root.display().to_string());

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_EXTERNAL_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_external_skills": ActiveExternalSkillsState {
                    skills: vec![ActiveExternalSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: vec!["shell.exec".to_owned()],
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

        let assembled = runtime
            .build_context(
                &config,
                session_id,
                true,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("build context");
        let system_content = assembled.messages[0]["content"]
            .as_str()
            .expect("system prompt should be text");

        assert!(
            system_content.contains("[active_external_skills]"),
            "expected active external skills marker, got: {system_content}"
        );
        assert!(
            system_content.contains("release-guard"),
            "expected skill id in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Release Guard"),
            "expected skill display name in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("protect releases"),
            "expected skill instructions in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Allowed tools: shell.exec"),
            "expected allowed tool summary in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Blocked tools: web.fetch"),
            "expected blocked tool summary in system prompt, got: {system_content}"
        );
    }

    #[tokio::test]
    async fn default_runtime_tool_view_excludes_active_external_skill_blocked_tools() {
        let runtime = DefaultConversationRuntime::default();
        let session_id = "session-active-external-skill-tool-block";
        let root = unique_temp_dir("active-external-skill-tool-block");
        let sqlite_path = root.join("memory.db");

        let mut config = LoongConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();

        let base_tool_view = crate::tools::runtime_tool_view_from_loong_config(&config);
        assert!(
            base_tool_view.contains("web"),
            "default runtime should expose the direct web surface"
        );

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_EXTERNAL_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_external_skills": ActiveExternalSkillsState {
                    skills: vec![ActiveExternalSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: Vec::new(),
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

        let tool_view = runtime
            .tool_view(&config, session_id, ConversationRuntimeBinding::direct())
            .expect("runtime tool view");

        assert!(
            !tool_view.contains("web"),
            "blocked hidden tool should also remove its direct surface"
        );
        assert!(
            tool_view.contains("read"),
            "unrelated direct tools should remain visible"
        );
    }
}
