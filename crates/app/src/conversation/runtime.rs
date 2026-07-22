use crate::CliResult;
use crate::runtime_self_continuity::RuntimeSelfContinuity;

use super::super::config::LoongConfig;
#[cfg(feature = "memory-sqlite")]
use super::active_skills;
#[cfg(test)]
use super::context_engine::ContextArtifactKind;
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
use super::context_engine_registry::resolve_context_engine;
use super::turn_engine::ProviderTurn;
use super::turn_middleware::{ConversationTurnMiddleware, builtin_turn_middlewares};
use super::turn_middleware_registry::resolve_turn_middlewares;
use super::{PromptFragment, PromptFrameAuthority, PromptLane};
#[cfg(test)]
use async_trait::async_trait;

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
#[cfg(feature = "memory-sqlite")]
use runtime_delegate::DefaultAsyncDelegateSpawner;
#[cfg(feature = "memory-sqlite")]
pub use runtime_delegate::execute_async_delegate_spawn_request;
pub use runtime_delegate::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner};
#[cfg(feature = "memory-sqlite")]
pub use runtime_hosted::HostedConversationRuntime;
#[cfg(test)]
use runtime_prompt::normalize_turn_middleware_ids;
pub use runtime_selection::{
    ContextCompactionPolicySnapshot, ContextEngineRuntimeSnapshot, ContextEngineSelection,
    ContextEngineSelectionSource, TurnMiddlewareRuntimeSnapshot, TurnMiddlewareSelection,
    TurnMiddlewareSelectionSource, collect_context_engine_runtime_snapshot,
    resolve_context_engine_selection, resolve_turn_middleware_selection,
};
pub use runtime_trait::ConversationRuntime;
#[cfg(test)]
use serde_json::Value;

pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
    turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
}

pub type BoxedDefaultConversationRuntime =
    DefaultConversationRuntime<Box<dyn ConversationContextEngine>>;

impl DefaultConversationRuntime<DefaultContextEngine> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            context_engine: DefaultContextEngine,
            turn_middlewares: builtin_turn_middlewares(),
        }
    }

    #[must_use]
    pub fn with_turn_middlewares(
        turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
    ) -> Self {
        let mut combined_turn_middlewares = builtin_turn_middlewares();
        combined_turn_middlewares.extend(turn_middlewares);
        Self {
            context_engine: DefaultContextEngine,
            turn_middlewares: combined_turn_middlewares,
        }
    }
}

impl Default for DefaultConversationRuntime<DefaultContextEngine> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> DefaultConversationRuntime<E> {
    #[must_use]
    pub fn with_context_engine(context_engine: E) -> Self {
        Self {
            context_engine,
            turn_middlewares: builtin_turn_middlewares(),
        }
    }

    #[must_use]
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
    pub fn context_engine_metadata(&self) -> ContextEngineMetadata {
        self.context_engine.metadata()
    }
}

impl DefaultConversationRuntime<Box<dyn ConversationContextEngine>> {
    pub fn from_engine_id(engine_id: Option<&str>) -> CliResult<Self> {
        let context_engine = resolve_context_engine(engine_id)?;
        Ok(Self {
            context_engine,
            turn_middlewares: builtin_turn_middlewares(),
        })
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
