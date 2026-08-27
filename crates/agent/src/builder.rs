//! Runtime-checked assembly for [`Agent`](super::Agent).
//!
//! The builder keeps capabilities at the [`Facade`] boundary and validates
//! that every registered tool set receives the resources it declares, such as
//! [`WorkspaceRoot`] for [`FileTools`]. Validation happens at [`spawn`](
//! AgentBuilder::spawn), so `with_*` methods stay infallible.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
    path::{Path, PathBuf},
    sync::Arc,
};

use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use loac::{ActorOwner, ActorRef, ExitStatus, Shutdown, ShutdownStatus, spawn};
use provider::Provider;
use tool_host::{RegistrationError, ToolRegistry};

use super::{Agent, ContextStore, ProviderOut};
use crate::channel::{ChannelTarget, ChannelTool};

/// A singleton resource provided to tools, keyed by its Rust type.
pub trait Resource: 'static + Send + Sync {
    /// Stable name used in error messages.
    const NAME: &'static str;
}

/// The workspace root available to filesystem tools.
pub struct WorkspaceRoot(pub PathBuf);

impl Resource for WorkspaceRoot {
    const NAME: &'static str = "WorkspaceRoot";
}

/// A type-keyed resource store.
///
/// Singleton resources such as [`WorkspaceRoot`] are inserted by type and
/// looked up by type. Named multi-instance resources such as agent channels are
/// registered as tools instead of living here.
#[derive(Default)]
pub struct Resources {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    pub fn insert<R: Resource>(&mut self, resource: R) {
        self.values.insert(TypeId::of::<R>(), Box::new(resource));
    }

    pub fn get<R: Resource>(&self) -> Option<&R> {
        self.values
            .get(&TypeId::of::<R>())
            .and_then(|value| value.downcast_ref::<R>())
    }

    fn contains(&self, type_id: TypeId) -> bool {
        self.values.contains_key(&type_id)
    }
}

/// A tool set's declared dependency on one resource type.
#[derive(Debug, Clone, Copy)]
pub struct ResourceNeed {
    type_id: TypeId,
    name: &'static str,
}

impl ResourceNeed {
    #[must_use]
    pub fn of<R: Resource>() -> Self {
        Self {
            type_id: TypeId::of::<R>(),
            name: R::NAME,
        }
    }
}

/// A collection of tools registered into one agent.
pub trait ToolSet: Send + Sync + 'static {
    /// Stable tool-set name used in error messages.
    fn name(&self) -> &'static str;

    /// Registers the concrete tools into the agent's registry.
    fn register(&self, registry: &mut ToolRegistry) -> Result<(), RegistrationError>;

    /// Declares the resources this tool set needs at spawn time.
    fn needs(&self) -> Vec<ResourceNeed> {
        Vec::new()
    }
}

/// The built-in filesystem tool set: `read_file` and `write_file`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileTools;

impl ToolSet for FileTools {
    fn name(&self) -> &'static str {
        "FileTools"
    }

    fn register(&self, registry: &mut ToolRegistry) -> Result<(), RegistrationError> {
        registry.register("read_file".to_owned(), tools::ReadFileTool)?;
        registry.register("write_file".to_owned(), tools::WriteFileTool)?;
        Ok(())
    }

    fn needs(&self) -> Vec<ResourceNeed> {
        vec![ResourceNeed::of::<WorkspaceRoot>()]
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
    #[error("tool registration failed: {0}")]
    Registration(#[from] RegistrationError),
}

/// Runtime-checked agent builder.
///
/// Capabilities enter through the [`Facade`] passed to
/// [`Agent::builder`](super::Agent::builder). Tool sets declare resource needs
/// and [`spawn`](Self::spawn) validates them before starting the actor.
pub struct AgentBuilder<C, P> {
    facade: Facade,
    resources: Resources,
    tools: Vec<Box<dyn ToolSet>>,
    channels: Vec<(&'static str, Arc<dyn ChannelTarget>)>,
    system_prompt: Option<String>,
    _marker: PhantomData<fn() -> (C, P)>,
}

impl<C, P> AgentBuilder<C, P> {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            resources: Resources::default(),
            tools: Vec::new(),
            channels: Vec::new(),
            system_prompt: None,
            _marker: PhantomData,
        }
    }

    #[must_use]
    pub fn with<T: ToolSet>(mut self, tools: T) -> Self {
        self.tools.push(Box::new(tools));
        self
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

    fn validate(&self) -> Result<(), BuildError> {
        for tool_set in &self.tools {
            for need in tool_set.needs() {
                if !self.resources.contains(need.type_id) {
                    return Err(BuildError::MissingResource {
                        resource: need.name,
                        tool_set: tool_set.name(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validates the assembled tools and resources, then starts the agent.
    pub fn spawn(self, store: C, provider: P) -> Result<AgentHandle<C, P>, BuildError>
    where
        C: ContextStore + 'static,
        P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    {
        self.validate()?;

        let Self {
            facade,
            resources,
            tools,
            channels,
            system_prompt,
            _marker,
        } = self;

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

        let owner =
            spawn::<Agent<C, P>>((store, provider, registry, workspace_root, system_prompt));
        let actor_ref = owner.actor_ref();

        Ok(AgentHandle { owner, actor_ref })
    }
}

/// Owned handle returned by [`AgentBuilder::spawn`].
pub struct AgentHandle<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    owner: ActorOwner<Agent<C, P>>,
    actor_ref: ActorRef<Agent<C, P>>,
}

impl<C, P> AgentHandle<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    #[must_use]
    pub fn actor_ref(&self) -> ActorRef<Agent<C, P>> {
        self.actor_ref.clone()
    }

    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.owner.request_shutdown(shutdown)
    }

    #[must_use]
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.owner.exit_status()
    }

    pub async fn wait(&mut self) -> ExitStatus {
        self.owner.wait().await
    }

    pub async fn shutdown(self, shutdown: Shutdown) -> ExitStatus {
        self.owner.shutdown(shutdown).await
    }
}
