use crate::CliResult;
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::ToolView;

use super::super::{config::LoongConfig, provider};
#[cfg(feature = "memory-sqlite")]
use super::active_skills;
#[cfg(test)]
use super::context_engine::ContextArtifactKind;
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
use super::context_engine_registry::resolve_context_engine;
use super::mailbox_for_session;
use super::prompt_orchestrator::seed_prompt_fragments_from_context;
use super::prompt_orchestrator::sync_prompt_fragments_into_context;
use super::runtime_binding::ConversationRuntimeBinding;
#[cfg(test)]
use super::runtime_binding::OwnedConversationRuntimeBinding;
#[cfg(test)]
use super::subagent::DelegateBuiltinProfile;
use super::turn_engine::ProviderTurn;
use super::turn_middleware::{ConversationTurnMiddleware, builtin_turn_middlewares};
use super::turn_middleware_registry::resolve_turn_middlewares;
use super::{PromptFragment, PromptFrameAuthority, PromptLane};
#[cfg(test)]
use async_trait::async_trait;

#[path = "runtime_context.rs"]
mod runtime_context;
#[path = "runtime_delegate.rs"]
mod runtime_delegate;
#[path = "runtime_hosted.rs"]
mod runtime_hosted;
#[path = "runtime_prompt.rs"]
mod runtime_prompt;
#[path = "runtime_selection.rs"]
mod runtime_selection;
#[path = "runtime_trait.rs"]
mod runtime_trait;
#[path = "runtime_turn_middleware.rs"]
mod runtime_turn_middleware;
#[path = "runtime_session.rs"]
mod session_runtime;
pub use runtime_context::SessionContext;
use runtime_context::{model_visible_skill_roots_from_config, root_session_context_from_config};
#[cfg(feature = "memory-sqlite")]
use runtime_delegate::DefaultAsyncDelegateSpawner;
#[cfg(feature = "memory-sqlite")]
pub use runtime_delegate::execute_async_delegate_spawn_request;
pub use runtime_delegate::{
    AsyncDelegateSpawnRequest, AsyncDelegateSpawner,
    async_delegate_spawn_request_from_serialized_parts,
};
#[cfg(feature = "memory-sqlite")]
pub use runtime_hosted::HostedConversationRuntime;
#[cfg(test)]
use runtime_prompt::normalize_turn_middleware_ids;
use runtime_prompt::{
    active_skills_prompt_summary, append_runtime_prompt_fragment,
    delegate_child_profile_prompt_summary, delegate_child_runtime_contract_prompt_summary,
    provider_runtime_binding, runtime_self_continuity_prompt_summary,
};
pub use runtime_selection::{
    ContextCompactionPolicySnapshot, ContextEngineRuntimeSnapshot, ContextEngineSelection,
    ContextEngineSelectionSource, TurnMiddlewareRuntimeSnapshot, TurnMiddlewareSelection,
    TurnMiddlewareSelectionSource, collect_context_engine_runtime_snapshot,
    resolve_context_engine_selection, resolve_turn_middleware_selection,
};
pub use runtime_trait::ConversationRuntime;
#[cfg(feature = "memory-sqlite")]
use session_runtime::{
    apply_active_skill_blocked_tools_to_tool_view, apply_session_tool_policy_to_tool_view,
    build_base_tool_view_from_snapshot, build_session_context_from_snapshot,
    load_persisted_session_context, load_persisted_session_snapshot, open_session_repository,
};

#[cfg(test)]
use crate::tools::runtime_config::ToolRuntimeNarrowing;
#[cfg(test)]
use serde_json::Value;

pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
    turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
}

pub type BoxedDefaultConversationRuntime =
    DefaultConversationRuntime<Box<dyn ConversationContextEngine>>;

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
        let active_skills = include_system_prompt
            .then(|| {
                active_skills_prompt_summary(effective_config, session_context.session_id.as_str())
            })
            .flatten();
        #[cfg(not(feature = "memory-sqlite"))]
        let active_skills: Option<String> = None;
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
            "active-skills",
            active_skills,
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
pub use runtime_hosted::load_hosted_default_conversation_runtime;

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
