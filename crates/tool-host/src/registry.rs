//! Tool registry: type-erased tool handles and the concrete [`ToolRegistry`]
//! host the agent invokes.

use std::{collections::BTreeMap, marker::PhantomData, path::Path};

use async_trait::async_trait;
use contracts::tool::ToolSpec;
use kernel::Facade;
use serde_json::Value;

use crate::tool::{
    InvocationParams, RegistrationError, ToolContext, ToolError, ToolHost, ToolImpl,
};

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
