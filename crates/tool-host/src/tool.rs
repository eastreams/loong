use std::{collections::BTreeMap, marker::PhantomData, path::Path};

use async_trait::async_trait;
use contracts::{capability::Capabilities, tool::ToolSpec};
use kernel::{Facade, access::fs::FsAccess};
use schemars::{JsonSchema, Schema};
use serde_json::Value;
use thiserror::Error;

/// Per-call ambient parameters supplied by the agent.
#[derive(Debug, Clone)]
pub struct InvocationParams {
    pub workspace_root: std::path::PathBuf,
    pub capabilities_override: Option<Capabilities>,
}

impl InvocationParams {
    #[must_use]
    pub fn new(
        workspace_root: impl AsRef<Path>,
        capabilities_override: Option<Capabilities>,
    ) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
            capabilities_override,
        }
    }
}

/// Why a tool could not be registered.
#[derive(Debug, Error)]
pub enum RegistrationError {
    #[error("duplicate tool {0:?}")]
    Duplicate(String),
    #[error("invalid tool spec")]
    InvalidSpec,
}

/// Why a tool invocation failed.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool {0:?}")]
    UnknownTool(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(#[source] serde_json::Error),
    #[error("tool execution failed: {0}")]
    Execution(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("tool output serialization failed: {0}")]
    Output(#[source] serde_json::Error),
}

/// Root JSON Schema for a tool input or output type.
fn root_schema<T: JsonSchema>() -> Schema {
    let settings = schemars::generate::SchemaSettings::default().with(|settings| {
        settings.meta_schema = None;
        settings.inline_subschemas = true;
    });
    let generator = settings.into_generator();
    generator.into_root_schema_for::<T>()
}

/// The trusted handle supplied to one tool invocation.
pub trait ToolContext<H: ToolHost>: Sync {
    fn facade(&self) -> &Facade;
    fn workspace_root(&self) -> &Path;

    fn fs(&self) -> FsAccess<'_> {
        FsAccess::new(self.facade(), self.workspace_root())
    }
}

/// The tool host boundary the agent calls.
#[async_trait]
pub trait ToolHost: Send + Sync + Sized + 'static {
    type ToolCx<'a>: ToolContext<Self>
    where
        Self: 'a;

    async fn invoke(
        &self,
        name: &str,
        params: &InvocationParams,
        payload: Value,
    ) -> Result<Value, ToolError>;
}

/// A concrete tool implementation.
#[async_trait]
pub trait ToolImpl<H: ToolHost>: Send + Sync + 'static {
    type Input: JsonSchema + serde::de::DeserializeOwned + Send + 'static;
    type Output: JsonSchema + serde::Serialize + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn name(&self) -> &'static str;

    fn description(&self) -> &'static str;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: self.description().into(),
            input_schema: root_schema::<Self::Input>(),
            output_schema: root_schema::<Self::Output>(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, serde_json::Error> {
        serde_json::from_value(payload)
    }

    async fn execute(
        &self,
        ctx: &H::ToolCx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error>;
}

/// Registered metadata for a tool.
#[derive(Debug, Clone)]
pub struct ToolRegistration {
    spec: ToolSpec,
}

impl ToolRegistration {
    fn new(spec: ToolSpec) -> Result<Self, RegistrationError> {
        if spec.name.is_empty() {
            return Err(RegistrationError::InvalidSpec);
        }
        Ok(Self { spec })
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }
}

#[async_trait]
trait ToolAdapter<H: ToolHost>: Send + Sync {
    async fn invoke(&self, ctx: &H::ToolCx<'_>, payload: Value) -> Result<Value, ToolError>;
}

/// Type-erased tool handle stored in the registry.
pub struct RegisteredTool<H: ToolHost> {
    registration: ToolRegistration,
    adapter: Box<dyn ToolAdapter<H>>,
}

impl<H: ToolHost> RegisteredTool<H> {
    pub fn from_tool<T: ToolImpl<H>>(tool: T) -> Result<Self, RegistrationError>
    where
        H: 'static,
    {
        let registration = ToolRegistration::new(tool.spec())?;
        let adapter = Box::new(CoreToolAdapter::new(tool));
        Ok(Self {
            registration,
            adapter,
        })
    }

    #[must_use]
    pub fn registration(&self) -> &ToolRegistration {
        &self.registration
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        self.registration.spec()
    }

    pub async fn invoke(&self, ctx: &H::ToolCx<'_>, payload: Value) -> Result<Value, ToolError> {
        self.adapter.invoke(ctx, payload).await
    }
}

struct CoreToolAdapter<H: ToolHost, T: ToolImpl<H>> {
    inner: T,
    _marker: PhantomData<fn() -> H>,
}

impl<H: ToolHost, T: ToolImpl<H>> CoreToolAdapter<H, T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }
}

#[async_trait]
impl<H, T> ToolAdapter<H> for CoreToolAdapter<H, T>
where
    H: ToolHost,
    T: ToolImpl<H>,
{
    async fn invoke(&self, ctx: &H::ToolCx<'_>, payload: Value) -> Result<Value, ToolError> {
        let input = self
            .inner
            .parse_input(payload)
            .map_err(ToolError::InvalidInput)?;
        let output = self
            .inner
            .execute(ctx, input)
            .await
            .map_err(|error| ToolError::Execution(Box::new(error)))?;
        serde_json::to_value(output).map_err(ToolError::Output)
    }
}

/// The concrete tool host for the loong agent.
pub struct ToolRegistry {
    facade: Facade,
    tools: BTreeMap<String, RegisteredTool<ToolRegistry>>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new(facade: Facade) -> Self {
        Self {
            facade,
            tools: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn facade(&self) -> &Facade {
        &self.facade
    }

    pub fn register<T: ToolImpl<Self>>(
        &mut self,
        name: String,
        tool: T,
    ) -> Result<(), RegistrationError> {
        if self.tools.contains_key(&name) {
            return Err(RegistrationError::Duplicate(name));
        }
        let registered = RegisteredTool::from_tool(tool)?;
        self.tools.insert(name, registered);
        Ok(())
    }

    #[must_use]
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|registered| registered.spec().clone())
            .collect()
    }

    pub async fn invoke(
        &self,
        name: &str,
        params: &InvocationParams,
        payload: Value,
    ) -> Result<Value, ToolError> {
        let registered = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::UnknownTool(name.to_owned()))?;
        let ctx = ToolRegistryContext {
            registry: self,
            workspace_root: &params.workspace_root,
        };
        registered.invoke(&ctx, payload).await
    }
}

/// Per-call tool context backed by the tool registry.
pub struct ToolRegistryContext<'a> {
    registry: &'a ToolRegistry,
    workspace_root: &'a Path,
}

impl ToolContext<ToolRegistry> for ToolRegistryContext<'_> {
    fn facade(&self) -> &Facade {
        self.registry.facade()
    }

    fn workspace_root(&self) -> &Path {
        self.workspace_root
    }
}

#[async_trait]
impl ToolHost for ToolRegistry {
    type ToolCx<'a>
        = ToolRegistryContext<'a>
    where
        Self: 'a;

    async fn invoke(
        &self,
        name: &str,
        params: &InvocationParams,
        payload: Value,
    ) -> Result<Value, ToolError> {
        ToolRegistry::invoke(self, name, params, payload).await
    }
}
