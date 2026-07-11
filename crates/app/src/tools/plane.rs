use std::{collections::BTreeMap, sync::OnceLock};

use async_trait::async_trait;
use loong_contracts::{ToolExecutionError, ToolInputError, ToolOutcome, ToolPath, ToolPlaneError};
use loong_core::{
    policy::context::ContextFactory,
    policy::grant::Granted,
    tool::{RegisteredTool, ToolImpl, ToolInvocationAction, ToolProvenance},
};

use crate::context::AppContextFactory;

/// App-owned typed tool dispatch plane.
///
/// The plane only resolves and executes app-registered tools. Kernel remains
/// responsible for authorization and audit, so callers resolve by path, request
/// a kernel grant, then call `invoke`. Payload parsing belongs to the selected
/// tool; the plane does not claim ownership of a payload shape.
#[async_trait]
pub(crate) trait ToolPlane<C: ContextFactory>: Send + Sync {
    fn contains(&self, path: &ToolPath) -> bool;

    async fn invoke(
        &self,
        grant: Granted<ToolInvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<ToolOutcome, ToolPlaneError>;
}

pub(crate) struct AppToolPlane<C: ContextFactory> {
    tools: BTreeMap<ToolPath, RegisteredTool<C>>,
}

impl<C> AppToolPlane<C>
where
    C: ContextFactory,
{
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub(crate) fn register<T>(&mut self, path: ToolPath, tool: T) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
    {
        self.register_with_provenance(path, ToolProvenance::Builtin, tool)
    }

    pub(crate) fn register_with_provenance<T>(
        &mut self,
        path: ToolPath,
        provenance: ToolProvenance,
        tool: T,
    ) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
    {
        if self.tools.contains_key(&path) {
            return Err(ToolPlaneError::DuplicateTool(path.to_string()));
        }

        self.tools
            .insert(path, RegisteredTool::from_tool(provenance, tool));
        Ok(())
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn contains(&self, path: &ToolPath) -> bool {
        self.tools.contains_key(path)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.tools.len()
    }
}

#[async_trait]
impl<C> ToolPlane<C> for AppToolPlane<C>
where
    C: ContextFactory,
{
    fn contains(&self, path: &ToolPath) -> bool {
        self.tools.contains_key(path)
    }

    /// This is not expected to be used directly.
    /// Use `ctx.invoke_tool()` in the future
    async fn invoke(
        &self,
        grant: Granted<ToolInvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<ToolOutcome, ToolPlaneError> {
        // Consuming the grant here makes audit/grant enforcement automatic for
        // concrete tool authors: ToolImpl implementers never receive a raw
        // dispatch path that can bypass app orchestration.
        let (path, _required_capabilities, payload) = grant.into_action().into_parts();
        let registered = self
            .tools
            .get(&path)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;

        registered
            .invoke(ctx, payload)
            .await
            .map_err(|error| ToolPlaneError::Execution(tool_execution_error_reason(error)))
    }
}

// TODO: make this in a unified struct
pub(crate) fn app_tool_plane() -> &'static dyn ToolPlane<AppContextFactory> {
    static TOOL_PLANE: OnceLock<AppToolPlane<AppContextFactory>> = OnceLock::new();
    TOOL_PLANE.get_or_init(|| {
        #[allow(unused_mut)]
        let mut plane = AppToolPlane::new();
        #[cfg(feature = "tool-file")]
        plane
            .register(ToolPath::from("read"), loong_tools::file::ReadFileTool)
            .expect("builtin app tools must register without duplicates");
        plane
    })
}

fn tool_execution_error_reason(error: ToolExecutionError) -> String {
    match error {
        ToolExecutionError::Input(ToolInputError::InvalidPayload { reason })
        | ToolExecutionError::Execution { reason } => reason,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
