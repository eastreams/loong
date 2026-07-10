use std::{borrow::Cow, collections::BTreeSet, time::SystemTime};

use async_trait::async_trait;
use loong_contracts::{
    Capability, ToolExecutionError, ToolInputError, ToolOutcome, ToolPath, ToolSpec,
};
use serde_json::{Value, json};

use crate::policy::{
    action::{ActionMeta, ActionMetadata},
    context::ContextFactory,
};

/// Policy action for allowing app orchestration to call one tool.
///
/// This action gates dispatch into a `ToolImpl`. It does not authorize the
/// side effects the tool may perform internally; those must still pass through
/// their own access actions such as filesystem read/write.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocationAction {
    path: ToolPath,
    required_capabilities: Vec<Capability>,
    payload: Value,
}

impl ToolInvocationAction {
    #[must_use]
    pub fn new(
        path: ToolPath,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
    ) -> Self {
        Self {
            path,
            required_capabilities: required_capabilities.into_iter().collect(),
            payload,
        }
    }

    #[must_use]
    pub fn path(&self) -> &ToolPath {
        &self.path
    }

    #[must_use]
    pub fn required_capabilities(&self) -> &[Capability] {
        self.required_capabilities.as_slice()
    }

    #[must_use]
    pub fn into_parts(self) -> (ToolPath, Vec<Capability>, Value) {
        (self.path, self.required_capabilities, self.payload)
    }
}

impl ActionMeta for ToolInvocationAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "tool.invoke",
            operation: Cow::Borrowed(self.path.as_str()),
            required_capabilities: Cow::Borrowed(self.required_capabilities.as_slice()),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "tool_path": self.path.as_str(),
            "payload": self.payload,
        }))
    }
}

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
    type Output: Send + Into<ToolOutcome> + 'static;

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
            erased: Box::new(tool),
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

    pub async fn invoke(
        &self,
        ctx: &C::Cx<'_>,
        payload: Value,
    ) -> Result<ToolOutcome, ToolExecutionError> {
        self.erased.invoke(ctx, payload).await
    }
}

#[async_trait]
trait ErasedTool<C: ContextFactory>: Send + Sync {
    async fn invoke(
        &self,
        ctx: &C::Cx<'_>,
        payload: Value,
    ) -> Result<ToolOutcome, ToolExecutionError>;
}

#[async_trait]
impl<C, T> ErasedTool<C> for T
where
    C: ContextFactory,
    T: ToolImpl<C>,
{
    async fn invoke(
        &self,
        ctx: &C::Cx<'_>,
        payload: Value,
    ) -> Result<ToolOutcome, ToolExecutionError> {
        let input = self.parse_input(payload)?;
        self.execute(ctx, input).await.map(Into::into)
    }
}

#[cfg(test)]
mod tests;
