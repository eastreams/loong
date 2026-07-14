use std::{error::Error, time::SystemTime};

use async_trait::async_trait;
use loong_contracts::{ToolInputError, ToolSpec};
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
    type Error: Error + Send + Sync + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError>;

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error>;
}

/// Failure produced while invoking a type-erased registered tool.
///
/// Parsing remains a stable core contract, while execution retains the concrete
/// tool error as its source so policy and orchestration boundaries can inspect
/// typed failures without teaching core about every tool crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegisteredToolError {
    #[error(transparent)]
    Input(#[from] ToolInputError),
    #[error("tool execution failed: {source}")]
    Execution {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
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
    /// belong. Returning `()` removes a recoverable error channel; it does not
    /// prevent a callback from panicking. Observers must not panic, and the
    /// registrar must absorb or handle recoverable delivery failures inside the
    /// callback. Fallible delivery needs a separate event contract rather than
    /// changing tool success after its side effects have completed.
    #[must_use]
    pub fn from_tool_with_success_observer<T, F>(
        provenance: ToolProvenance,
        tool: T,
        observer: F,
    ) -> Self
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) + Send + Sync + 'static,
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
    ) -> Result<Value, RegisteredToolError> {
        self.erased.invoke(ctx, payload).await
    }
}

// Private by design: callers may register concrete ToolImpl values, but cannot
// inject an erased implementation that bypasses input parsing or loses concrete
// error sources. Grant and audit remain the invoking ToolPlane's responsibility.
#[async_trait]
trait ErasedTool<C: ContextFactory>: Send + Sync {
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, RegisteredToolError>;
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
    F: for<'a> Fn(&C::Cx<'a>, &T::Output) + Send + Sync + 'static,
{
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, RegisteredToolError> {
        let input = self.tool.parse_input(payload)?;
        let output = self.tool.execute(ctx, input).await.map_err(|source| {
            RegisteredToolError::Execution {
                source: Box::new(source),
            }
        })?;
        (self.observer)(ctx, &output);
        Ok(output.into())
    }
}

#[async_trait]
impl<C, T> ErasedTool<C> for PlainTool<T>
where
    C: ContextFactory,
    T: ToolImpl<C>,
{
    async fn invoke(&self, ctx: &C::Cx<'_>, payload: Value) -> Result<Value, RegisteredToolError> {
        let input = self.tool.parse_input(payload)?;
        self.tool
            .execute(ctx, input)
            .await
            .map(Into::into)
            .map_err(|source| RegisteredToolError::Execution {
                source: Box::new(source),
            })
    }
}

#[cfg(test)]
mod tests;
