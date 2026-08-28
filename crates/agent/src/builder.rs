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
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use kernel::resource::{Resources, WorkspaceRoot};
use loac::{ActorScope, HasChildren};
use provider::Provider;
use tool_host::{RegistrationError, ToolRegistry};

use super::{Agent, ContextStore, ProviderOut};
use crate::channel::ChannelTarget;
use crate::channel_tool::ChannelTool;
use crate::tool_set::ToolSet;

/// Spawns one child agent and registers it as a named channel tool.
///
/// The child type is erased behind this trait so one parent can own child
/// agents with different store/provider types. Each child is started in the
/// parent actor's [`init`](loac::Actor::init), which gives the parent runtime
/// ownership of the child lifetime.
pub(crate) trait SubagentSpawner<C, P>: Send
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn name(&self) -> &str;

    fn spawn(self: Box<Self>, scope: &mut ActorScope<'_, Agent<C, P>>, registry: &mut ToolRegistry);
}

/// [`SubagentSpawner`] for a fully built child [`Agent`].
struct ChannelSubagent<C2, P2>
where
    C2: ContextStore,
    P2: Provider<Request, StreamItem, ProviderOut> + Clone,
{
    name: String,
    agent: Agent<C2, P2>,
}

impl<C, P, C2, P2> SubagentSpawner<C, P> for ChannelSubagent<C2, P2>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    C2: ContextStore + 'static,
    P2: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    Agent<C, P>: HasChildren,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn spawn(
        self: Box<Self>,
        scope: &mut ActorScope<'_, Agent<C, P>>,
        registry: &mut ToolRegistry,
    ) {
        let this = *self;
        let child = scope
            .spawn_child::<Agent<C2, P2>>(this.agent)
            .unwrap_or_else(|_| unreachable!("unbounded children accept every subagent"));
        let target: Arc<dyn ChannelTarget> = Arc::new(child.into_actor_ref());
        let tool = ChannelTool::new(this.name.clone(), target);
        registry
            .register(this.name, tool)
            .expect("builder validated unique subagent channel name");
    }
}

/// Why agent assembly failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("missing resource `{resource}` required by `{tool_set}`")]
    MissingResource {
        resource: &'static str,
        tool_set: &'static str,
    },
    #[error("duplicate channel or subagent {0:?}")]
    DuplicateChannel(String),
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
    resources: Resources,
    tools: Vec<Box<dyn ToolSet>>,
    channels: Vec<(String, Arc<dyn ChannelTarget>)>,
    subagents: Vec<Box<dyn SubagentSpawner<C, P>>>,
    system_prompt: Option<String>,
    store: Option<C>,
    provider: Option<P>,
}

impl<C, P> AgentBuilder<C, P, false, false> {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            resources: Resources::new(),
            tools: Vec::new(),
            channels: Vec::new(),
            subagents: Vec::new(),
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
            subagents: self.subagents,
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
            subagents: self.subagents,
            system_prompt: self.system_prompt,
            store: self.store,
            provider: Some(provider),
        }
    }

    /// Registers one named channel as a tool. The channel name is the tool
    /// name the model sees.
    #[must_use]
    pub fn with_channel(mut self, name: impl Into<String>, target: Arc<dyn ChannelTarget>) -> Self {
        self.channels.push((name.into(), target));
        self
    }

    /// Registers one fully built child agent as a named channel tool.
    ///
    /// The child is spawned inside the parent actor's init, so the parent
    /// runtime owns its lifetime and shuts it down with the parent. The child
    /// keeps its own store, provider, tools, and resources.
    #[must_use]
    pub fn with_subagent<C2, P2>(mut self, name: impl Into<String>, agent: Agent<C2, P2>) -> Self
    where
        C: ContextStore + 'static,
        P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
        C2: ContextStore + 'static,
        P2: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    {
        self.subagents.push(Box::new(ChannelSubagent {
            name: name.into(),
            agent,
        }));
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

    /// Resource needs are currently checked against a [`Resources`] value at
    /// runtime because `ToolSet::needs()` reports them as values. Once Rust
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
            subagents,
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
        for subagent in &subagents {
            let name = subagent.name().to_owned();
            if !names.insert(name.clone()) {
                return Err(BuildError::DuplicateChannel(name));
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
            subagents,
            system_prompt,
            prompt_queue: VecDeque::new(),
            active: None,
        })
    }
}
