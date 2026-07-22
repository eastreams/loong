//! Runtime-private tool registration and type erasure.
//!
//! Concrete tools implement [`ToolImpl`], while this module owns the only raw
//! erased dispatch port. Keeping the registered entry private makes
//! [`super::ToolInvocation`] the only caller that can cross from a grant into
//! tool execution.

use std::error::Error;

use async_trait::async_trait;
use loong_contracts::{ToolInputError, ToolSpec};
use loong_core::{
    policy::context::ContextFactory,
    tool::{ToolFailureKind, ToolImpl},
};
use serde_json::Value;

use super::ToolRegistration;

/// Failure produced while invoking a runtime-erased registered tool.
///
/// Parsing remains a stable tool contract, while execution retains the
/// concrete error as its source for typed orchestration decisions.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegisteredToolError {
    #[error(transparent)]
    Input(#[from] ToolInputError),
    #[error("tool execution denied: {source}")]
    Denied {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("tool execution failed: {source}")]
    Execution {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl RegisteredToolError {
    /// Preserve the concrete source while crossing the erased runtime boundary.
    fn from_execution<E>(kind: ToolFailureKind, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        let source = Box::new(source);
        match kind {
            ToolFailureKind::Denied => Self::Denied { source },
            ToolFailureKind::Execution => Self::Execution { source },
        }
    }
}

pub(crate) struct RegisteredTool<C: ContextFactory> {
    registration: ToolRegistration,
    spec: ToolSpec,
    erased: Box<dyn ErasedTool<C>>,
}

impl<C> RegisteredTool<C>
where
    C: ContextFactory,
{
    pub(super) fn from_tool<T>(registration: ToolRegistration, tool: T) -> Self
    where
        T: ToolImpl<C>,
    {
        Self {
            registration,
            spec: tool.spec(),
            erased: Box::new(PlainTool { tool }),
        }
    }

    /// Register a typed success observer owned by the app bootstrap boundary.
    ///
    /// The observer runs after concrete execution but before terminal audit. It
    /// is a non-authoritative app side channel and therefore cannot return an
    /// error that rewrites the tool result. Like all production runtime code,
    /// the callback must not panic; release builds abort rather than unwind.
    pub(super) fn from_tool_with_success_observer<T, F>(
        registration: ToolRegistration,
        tool: T,
        observer: F,
    ) -> Self
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) + Send + Sync + 'static,
    {
        Self {
            registration,
            spec: tool.spec(),
            erased: Box::new(ObservedTool { tool, observer }),
        }
    }

    pub(crate) fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    pub(crate) fn registration(&self) -> &ToolRegistration {
        &self.registration
    }

    pub(super) async fn invoke(
        &self,
        ctx: &C::Cx<'_>,
        payload: Value,
    ) -> Result<Value, RegisteredToolError> {
        self.erased.invoke(ctx, payload).await
    }
}

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
            let kind = self.tool.failure_kind(&source);
            RegisteredToolError::from_execution(kind, source)
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
            .map_err(|source| {
                let kind = self.tool.failure_kind(&source);
                RegisteredToolError::from_execution(kind, source)
            })
    }
}

#[cfg(test)]
mod tests;
