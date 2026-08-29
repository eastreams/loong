//! Runtime-checked assembly for [`Agent`](super::Agent).

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use kernel::resource::{Resources, WorkspaceRoot};
use provider::Provider;
use tool_host::{RegistrationError, ToolRegistry};

use super::{Agent, AgentProvider, ContextStore, ProviderOut};
use crate::channel::ChannelTarget;
use crate::channel_tool::ChannelTool;
use crate::tool_set::ToolSet;

/// Why agent assembly failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("missing resource `{resource}` required by `{tool_set}`")]
    MissingResource {
        resource: &'static str,
        tool_set: &'static str,
    },
    #[error("duplicate channel {0:?}")]
    DuplicateChannel(String),
    #[error("tool registration failed: {0}")]
    Registration(#[from] RegistrationError),
}

/// Runtime-checked agent builder.
pub struct AgentBuilder<const STORE_SET: bool = false, const PROVIDER_SET: bool = false> {
    facade: Facade,
    resources: Resources,
    tools: Vec<Box<dyn ToolSet>>,
    channels: Vec<(String, Arc<dyn ChannelTarget>)>,
    system_prompt: Option<String>,
    store: Option<Box<dyn ContextStore>>,
    provider: Option<AgentProvider>,
}

impl AgentBuilder<false, false> {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            resources: Resources::new(),
            tools: Vec::new(),
            channels: Vec::new(),
            system_prompt: None,
            store: None,
            provider: None,
        }
    }
}

impl<const STORE_SET: bool, const PROVIDER_SET: bool> AgentBuilder<STORE_SET, PROVIDER_SET> {
    #[must_use]
    pub fn with<T: ToolSet>(mut self, tools: T) -> Self {
        self.tools.push(Box::new(tools));
        self
    }

    /// Sets the agent's context store and marks it present in the builder type.
    #[must_use]
    pub fn with_store(
        self,
        store: impl ContextStore + 'static,
    ) -> AgentBuilder<true, PROVIDER_SET> {
        AgentBuilder {
            facade: self.facade,
            resources: self.resources,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: Some(Box::new(store)),
            provider: self.provider,
        }
    }

    /// Sets the agent's provider and marks it present in the builder type.
    #[must_use]
    pub fn with_provider<P>(self, provider: P) -> AgentBuilder<STORE_SET, true>
    where
        P: Provider<Request, StreamItem, ProviderOut> + 'static,
    {
        AgentBuilder {
            facade: self.facade,
            resources: self.resources,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: self.store,
            provider: Some(Arc::new(provider)),
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
        self.resources
            .insert(WorkspaceRoot(root.as_ref().to_path_buf()));
        self
    }

    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    fn validate(&self) -> Result<(), BuildError> {
        for tool_set in &self.tools {
            for need in tool_set.needs() {
                if !need.is_satisfied_by(&self.resources) {
                    return Err(BuildError::MissingResource {
                        resource: need.name(),
                        tool_set: tool_set.name(),
                    });
                }
            }
        }
        Ok(())
    }
}

impl AgentBuilder<true, true> {
    /// Validates the assembled tools and resources, then builds the actor.
    pub fn build(self) -> Result<Agent, BuildError> {
        self.validate()?;

        let Self {
            facade,
            resources,
            tools,
            channels,
            system_prompt,
            store,
            provider,
        } = self;

        let mut names = BTreeSet::new();
        for (name, _) in &channels {
            if !names.insert(name.clone()) {
                return Err(BuildError::DuplicateChannel(name.clone()));
            }
        }

        let store = store.expect("STORE_SET=true guarantees a store");
        let provider = provider.expect("PROVIDER_SET=true guarantees a provider");

        let mut resources = resources;
        if !resources.contains::<WorkspaceRoot>() {
            resources.insert(WorkspaceRoot(PathBuf::from(".")));
        }
        let facade = facade.with_resources(resources);

        let mut registry = ToolRegistry::new(facade);
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
        })
    }
}
