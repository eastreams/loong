//! Serializable agent configuration.
//!
//! [`AgentConfig`] is the declarative description of one agent. It compiles
//! into the provider-agnostic [`AgentBuilder`] and then
//! into an [`Agent`].

use std::{collections::HashMap, fmt, path::PathBuf, sync::Arc};

use agent::{Agent, AgentBuilder, BuildError, ChannelTarget, FileTools, ProviderSet, StoreSet};
use context::disk::{DiskStore, OpenError as DiskStoreOpenError};
use context::memory::MemoryStore;
use contracts::capability::Capabilities;
use kernel::Facade;
use loac::{ActorOwner, ActorRef};
use provider_openai::{OpenAiConfig, OpenAiProvider};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Declarative description of one agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Stable diagnostic name, used in build errors.
    pub name: String,
    /// Optional system prompt prepended to every request.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Workspace root for file tools.
    #[serde(default = "default_workspace")]
    pub workspace_root: PathBuf,
    /// Capabilities requested for the agent's facade.
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Context store backend.
    pub store: StoreConfig,
    /// Upstream provider.
    pub provider: ProviderConfig,
    /// Built-in tool sets to register.
    #[serde(default)]
    pub tools: Vec<ToolConfig>,
    /// Channel names to register as tools. Targets are resolved at build time
    /// from the supplied channel map.
    #[serde(default)]
    pub channels: Vec<String>,
}

fn default_workspace() -> PathBuf {
    PathBuf::from(".")
}

/// Context store backend selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoreConfig {
    /// Ephemeral in-memory store.
    Memory,
    /// Durable disk-backed store rooted at `path`.
    Disk { path: PathBuf },
}

/// Provider selection.
///
/// `OpenAi` is the serializable production variant. `Resolved` carries an
/// already-built provider for embedders and tests and is skipped by serde.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    OpenAi(OpenAiConfig),
    #[serde(skip)]
    Resolved(agent::AgentProvider),
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAi(config) => formatter.debug_tuple("OpenAi").field(config).finish(),
            Self::Resolved(_) => formatter.write_str("Resolved(..)"),
        }
    }
}

impl ProviderConfig {
    /// Resolves this config into a concrete provider.
    pub fn to_provider(&self) -> agent::AgentProvider {
        match self {
            Self::OpenAi(config) => Arc::new(OpenAiProvider::new(config.clone())),
            Self::Resolved(provider) => Arc::clone(provider),
        }
    }
}

/// Built-in tool set selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolConfig {
    /// `read_file` and `write_file`.
    FileTools,
}

/// Why building an agent from an [`AgentConfig`] failed.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// A configured channel has no runtime target.
    #[error("unknown channel {0:?}")]
    MissingChannel(String),
    /// The underlying agent builder rejected the assembly.
    #[error("agent build failed: {0}")]
    Build(#[from] BuildError),
    /// The configured disk store could not be opened.
    #[error("disk store open failed: {0}")]
    Store(#[from] DiskStoreOpenError),
}

impl AgentConfig {
    /// Compiles this config into a builder using the supplied facade as the
    /// security ceiling.
    ///
    /// `channels` resolves the configured channel names to live targets.
    pub fn into_builder(
        &self,
        facade: Facade,
        channels: &HashMap<String, Arc<dyn ChannelTarget>>,
    ) -> Result<AgentBuilder<StoreSet, ProviderSet>, ConfigError> {
        let mut builder = Agent::builder(facade).with_workspace_root(&self.workspace_root);
        if let Some(prompt) = &self.system_prompt {
            builder = builder.with_system_prompt(prompt.clone());
        }

        let builder = match &self.store {
            StoreConfig::Memory => builder.with_store(MemoryStore::new()),
            StoreConfig::Disk { path } => builder.with_store(DiskStore::open(path.clone())?),
        };
        let builder = builder.with_provider(self.provider.to_provider());

        let mut builder = builder;
        for tool in &self.tools {
            match tool {
                ToolConfig::FileTools => builder = builder.with(FileTools),
            }
        }
        for name in &self.channels {
            let target = channels
                .get(name)
                .ok_or_else(|| ConfigError::MissingChannel(name.clone()))?;
            builder = builder.with_channel(name.clone(), Arc::clone(target));
        }

        Ok(builder)
    }

    /// Builds an [`Agent`] using the supplied facade as the security ceiling.
    pub fn build(
        &self,
        facade: Facade,
        channels: &HashMap<String, Arc<dyn ChannelTarget>>,
    ) -> Result<Agent, ConfigError> {
        Ok(self.into_builder(facade, channels)?.build()?)
    }

    /// Builds and spawns an [`Agent`] using the supplied facade.
    pub fn spawn(
        &self,
        facade: Facade,
        channels: &HashMap<String, Arc<dyn ChannelTarget>>,
    ) -> Result<ActorOwner<Agent>, ConfigError> {
        Ok(self.build(facade, channels)?.spawn())
    }

    /// Convenience for top-level agents: builds the facade from `kernel` and
    /// the configured capabilities, then spawns.
    pub fn spawn_from_kernel(
        &self,
        kernel: ActorRef<kernel::Kernel>,
        channels: &HashMap<String, Arc<dyn ChannelTarget>>,
    ) -> Result<ActorOwner<Agent>, ConfigError> {
        self.spawn(Facade::new(kernel, self.capabilities), channels)
    }
}
