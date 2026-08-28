//! Runtime-checked assembly for [`Agent`](super::Agent).
//!
//! The builder keeps capabilities at the [`Facade`] boundary and validates
//! that every registered tool set receives the resources it declares, such as
//! [`WorkspaceRoot`] for [`FileTools`]. Validation happens at [`build`](
//! AgentBuilder::build), so `with_*` methods stay infallible.
//!
//! The builder also type-encodes the required actor state: `STORE_SET` and
//! `PROVIDER_SET` advance as [`with_store`](AgentBuilder::with_store) and
//! [`with_provider`](AgentBuilder::with_provider) are called, and
//! [`build`](AgentBuilder::build) only exists once both are `true`.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

use anymap2::AnyMap;
use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use provider::Provider;
use tool_host::{RegistrationError, ToolRegistry};

use super::{Agent, ContextStore, ProviderOut};
use crate::channel::ChannelTarget;
use crate::channel_tool::ChannelTool;
use crate::resource::WorkspaceRoot;
use crate::tool_set::ToolSet;

/// Why agent assembly failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("missing resource `{resource}` required by `{tool_set}`")]
    MissingResource {
        resource: &'static str,
        tool_set: &'static str,
    },
    #[error("tool registration failed: {0}")]
    Registration(#[from] RegistrationError),
}

/// Runtime-checked agent builder.
///
/// Capabilities enter through the [`Facade`] passed to
/// [`Agent::builder`](super::Agent::builder). Tool sets declare resource needs
/// and [`spawn`](Self::spawn) validates them before starting the actor.
///
/// `C` and `P` are inferred by [`with_store`](Self::with_store) and
/// [`with_provider`](Self::with_provider). The `const bool` parameters track
/// whether the required store and provider are present, so `spawn` only
/// type-checks after both are set.
pub struct AgentBuilder<C, P, const STORE_SET: bool = false, const PROVIDER_SET: bool = false> {
    facade: Facade,
    resources: AnyMap,
    tools: Vec<Box<dyn ToolSet>>,
    channels: Vec<(&'static str, Arc<dyn ChannelTarget>)>,
    system_prompt: Option<String>,
    store: Option<C>,
    provider: Option<P>,
}

impl<C, P> AgentBuilder<C, P, false, false> {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            resources: AnyMap::new(),
            tools: Vec::new(),
            channels: Vec::new(),
            system_prompt: None,
            store: None,
            provider: None,
        }
    }
}

impl<C, P, const STORE_SET: bool, const PROVIDER_SET: bool>
    AgentBuilder<C, P, STORE_SET, PROVIDER_SET>
{
    #[must_use]
    pub fn with<T: ToolSet>(mut self, tools: T) -> Self {
        self.tools.push(Box::new(tools));
        self
    }

    /// Sets the agent's context store and marks it present in the builder type.
    #[must_use]
    pub fn with_store(self, store: C) -> AgentBuilder<C, P, true, PROVIDER_SET> {
        AgentBuilder {
            facade: self.facade,
            resources: self.resources,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: Some(store),
            provider: self.provider,
        }
    }

    /// Sets the agent's provider and marks it present in the builder type.
    #[must_use]
    pub fn with_provider(self, provider: P) -> AgentBuilder<C, P, STORE_SET, true> {
        AgentBuilder {
            facade: self.facade,
            resources: self.resources,
            tools: self.tools,
            channels: self.channels,
            system_prompt: self.system_prompt,
            store: self.store,
            provider: Some(provider),
        }
    }

    /// Registers one named channel as a tool. The channel name is the tool
    /// name the model sees.
    #[must_use]
    pub fn with_channel(mut self, name: &'static str, target: Arc<dyn ChannelTarget>) -> Self {
        self.channels.push((name, target));
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

    /// Resource needs are currently checked against an [`AnyMap`] at runtime
    /// because `ToolSet::needs()` reports them as values. Once Rust
    /// specialization stabilizes, resource requirements can be lifted to the
    /// type level and checked at compile time, just like `STORE_SET` and
    /// `PROVIDER_SET`.
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

impl<C, P> AgentBuilder<C, P, true, true>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    /// Validates the assembled tools and resources, then builds the actor.
    pub fn build(self) -> Result<Agent<C, P>, BuildError> {
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

        let store = store.expect("STORE_SET=true guarantees a store");
        let provider = provider.expect("PROVIDER_SET=true guarantees a provider");

        let mut registry = ToolRegistry::new(facade);
        for tool_set in &tools {
            tool_set.register(&mut registry)?;
        }
        for (name, target) in &channels {
            registry.register(
                (*name).to_owned(),
                ChannelTool::new(name, Arc::clone(target)),
            )?;
        }

        let workspace_root = resources
            .get::<WorkspaceRoot>()
            .map(|root| root.0.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        Ok(Agent {
            store,
            provider,
            registry,
            workspace_root,
            system_prompt,
            prompt_queue: VecDeque::new(),
            active: None,
        })
    }
}
