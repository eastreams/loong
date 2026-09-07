//! Runtime-checked assembly for [`Agent`](super::Agent).

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use provider::Provider;
use tool_host::{RegistrationError, ToolRegistry};

use super::{Agent, AgentProvider, ContextStore, ProviderOut};
use crate::channel::ChannelTarget;
use crate::channel_tool::ChannelTool;
use crate::tool_set::ToolSet;

/// Why agent assembly failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("duplicate channel {0:?}")]
    DuplicateChannel(String),
    #[error("tool registration failed: {0}")]
    Registration(#[from] RegistrationError),
}

/// Builder state: no context store has been set yet.
pub struct StoreUnset;

/// Builder state: the context store slot is filled.
pub struct StoreSet(Box<dyn ContextStore>);

/// Builder state: no provider has been set yet.
pub struct ProviderUnset;

/// Builder state: the provider slot is filled.
pub struct ProviderSet(AgentProvider);

/// Runtime-checked agent builder.
///
/// The store and provider slots are tracked in the type: they are `*Unset`
/// until the matching `with_*` call fills them, and only a builder with both
/// slots filled can be [`build`](Self::build).
pub struct AgentBuilder<S = StoreUnset, P = ProviderUnset> {
    facade: Facade,
    workspace_root: PathBuf,
    tools: Vec<Box<dyn ToolSet>>,
    channels: Vec<(String, Arc<dyn ChannelTarget>)>,
    system_prompt: Option<String>,
    store: S,
    provider: P,
}

impl AgentBuilder<StoreUnset, ProviderUnset> {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            workspace_root: PathBuf::from("."),
            tools: Vec::new(),
            channels: Vec::new(),
            system_prompt: None,
            store: StoreUnset,
            provider: ProviderUnset,
        }
    }
}

impl<S, P> AgentBuilder<S, P> {
    #[must_use]
    pub fn with<T: ToolSet>(mut self, tools: T) -> Self {
        self.tools.push(Box::new(tools));
        self
    }

    /// Fills the builder's context store slot.
    #[must_use]
    pub fn with_store(self, store: impl ContextStore + 'static) -> AgentBuilder<StoreSet, P> {
        AgentBuilder {
            facade: self.facade,
            workspace_root: self.workspace_root,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: StoreSet(Box::new(store)),
            provider: self.provider,
        }
    }

    /// Fills the builder's provider slot.
    #[must_use]
    pub fn with_provider<T>(self, provider: T) -> AgentBuilder<S, ProviderSet>
    where
        T: Provider<Request, StreamItem, ProviderOut> + 'static,
    {
        AgentBuilder {
            facade: self.facade,
            workspace_root: self.workspace_root,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: self.store,
            provider: ProviderSet(Arc::new(provider)),
        }
    }

    /// Registers one named channel as a tool. The channel name is the tool
    /// name the model sees.
    #[must_use]
    pub fn with_channel(mut self, name: impl Into<String>, target: Arc<dyn ChannelTarget>) -> Self {
        self.channels.push((name.into(), target));
        self
    }

    #[must_use]
    pub fn with_workspace_root(mut self, root: impl AsRef<Path>) -> Self {
        self.workspace_root = root.as_ref().to_path_buf();
        self
    }

    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }
}

impl AgentBuilder<StoreSet, ProviderSet> {
    /// Builds the actor.
    pub fn build(self) -> Result<Agent, BuildError> {
        let Self {
            facade,
            workspace_root,
            tools,
            channels,
            system_prompt,
            store: StoreSet(store),
            provider: ProviderSet(provider),
        } = self;

        let mut names = BTreeSet::new();
        for (name, _) in &channels {
            if !names.insert(name.clone()) {
                return Err(BuildError::DuplicateChannel(name.clone()));
            }
        }

        let mut registry = ToolRegistry::new(facade, workspace_root);
        for tool_set in &tools {
            tool_set.register(&mut registry)?;
        }
        for (name, target) in &channels {
            registry.register(
                name.clone(),
                ChannelTool::new(name.clone(), Arc::clone(target)),
            )?;
        }

        Ok(Agent {
            store,
            provider,
            registry,
            system_prompt,
            prompt_queue: VecDeque::new(),
            active: None,
            draining: false,
        })
    }
}
