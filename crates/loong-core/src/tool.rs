use std::time::SystemTime;

use async_trait::async_trait;
use loong_contracts::{ToolExecutionError, ToolInputError, ToolSpec};
use serde_json::Value;

use crate::policy::context::ContextFactory;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProvenance {
    Builtin,
    Extension,
    Discovered,
    Compatibility,
}

#[async_trait]
pub trait ToolImpl<C: ContextFactory>: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + Into<Value> + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError>;

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRegistration {
    spec: ToolSpec,
    registered_at: SystemTime,
    provenance: ToolProvenance,
}

impl ToolRegistration {
    #[must_use]
    pub fn new(spec: ToolSpec, provenance: ToolProvenance) -> Self {
        Self::with_registered_at(spec, SystemTime::now(), provenance)
    }

    #[must_use]
    pub fn with_registered_at(
        spec: ToolSpec,
        registered_at: SystemTime,
        provenance: ToolProvenance,
    ) -> Self {
        Self {
            spec,
            registered_at,
            provenance,
        }
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    #[must_use]
    pub fn registered_at(&self) -> SystemTime {
        self.registered_at
    }

    #[must_use]
    pub fn provenance(&self) -> &ToolProvenance {
        &self.provenance
    }
}

pub struct RegisteredTool<C: ContextFactory> {
    registration: ToolRegistration,
    erased: Box<dyn ErasedTool<C>>,
}

impl<C> RegisteredTool<C>
where
    C: ContextFactory,
{
    #[must_use]
    pub fn from_tool<T>(provenance: ToolProvenance, tool: T) -> Self
    where
        T: ToolImpl<C>,
    {
        let registration = ToolRegistration::new(tool.spec(), provenance);
        Self {
            registration,
            erased: Box::new(PlainTool { tool }),
        }
    }

    /// Register a tool with a typed success observer owned by the registrar.
    ///
    /// Concrete tool crates still only implement [`ToolImpl`]. This hook lets
    /// the app/runtime boundary observe the concrete output before it is erased
    /// into JSON, which is where app-owned side channels such as preview events
    /// belong.
    #[must_use]
    pub fn from_tool_with_success_observer<T, F>(
        provenance: ToolProvenance,
        tool: T,
        observer: F,
    ) -> Self
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) -> Result<(), ToolExecutionError>
            + Send
            + Sync
            + 'static,
    {
        let registration = ToolRegistration::new(tool.spec(), provenance);
        Self {
            registration,
            erased: Box::new(ObservedTool { tool, observer }),
        }
    }

    #[must_use]
    pub fn registration(&self) -> &ToolRegistration {
        &self.registration
    }

    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        self.registration.spec()
    }

    /// Invoke the concrete tool and return its typed success payload.
    ///
    /// Legacy `"ok"/payload` envelopes belong to app bridge code that still
    /// speaks [`loong_contracts::ToolCoreOutcome`], not to erased tools.
    pub async fn invoke(
        &self,
        ctx: &C::Cx<'_>,
        payload: Value,
    ) -> Result<Value, ToolExecutionError> {
        self.erased.invoke(ctx, payload).await
    }
}

// Private by design: only RegisteredTool may erase concrete tool types. That
// keeps registration metadata attached to every dispatch path and prevents
// concrete ToolImpl authors from bypassing the app plane's grant/audit wrapper.
#[async_trait]
trait ErasedTool<C: ContextFactory>: Send + Sync {
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, ToolExecutionError>;
}

struct ObservedTool<T, F> {
    tool: T,
    observer: F,
}

struct PlainTool<T> {
    tool: T,
}

#[async_trait]
impl<C, T, F> ErasedTool<C> for ObservedTool<T, F>
where
    C: ContextFactory,
    T: ToolImpl<C>,
    F: for<'a> Fn(&C::Cx<'a>, &T::Output) -> Result<(), ToolExecutionError> + Send + Sync + 'static,
{
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, ToolExecutionError> {
        let input = self.tool.parse_input(payload)?;
        let output = self.tool.execute(ctx, input).await?;
        (self.observer)(ctx, &output)?;
        Ok(output.into())
    }
}

#[async_trait]
impl<C, T> ErasedTool<C> for PlainTool<T>
where
    C: ContextFactory,
    T: ToolImpl<C>,
{
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, ToolExecutionError> {
        let input = self.tool.parse_input(payload)?;
        self.tool.execute(ctx, input).await.map(Into::into)
    }
}

#[cfg(test)]
mod tests;
