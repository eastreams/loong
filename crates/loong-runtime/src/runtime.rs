//! Long-lived owner for governance and runtime registries.

use loong_contracts::{Capabilities, ToolPath, ToolSpec};
use loong_core::policy::context::ContextFactory;
use loong_kernel::Kernel;

use crate::tool_plane::{
    ToolInvocation, ToolInvocationContext, ToolPlane, ToolPlaneRegistry, error::LookupError,
};

/// Owns the kernel and tool plane shared by app sessions and execution contexts.
///
/// This is the runtime authority root, not a kernel facade or UI state object.
/// The concrete context remains app-defined through `C`. Registry storage is
/// erased only inside this crate so raw granted dispatch cannot become an
/// external extension point.
pub struct Runtime<C: ContextFactory> {
    kernel: Kernel<C>,
    tools: Box<dyn ToolPlane<C> + 'static>,
}

impl<C> Runtime<C>
where
    C: ContextFactory,
{
    pub fn new(kernel: Kernel<C>, tools: ToolPlaneRegistry<C>) -> Self {
        Self {
            kernel,
            tools: Box::new(tools),
        }
    }

    #[must_use]
    pub fn kernel(&self) -> &Kernel<C> {
        &self.kernel
    }

    #[must_use]
    pub fn registered_tool_paths(&self) -> Vec<ToolPath> {
        self.tools.registered_paths()
    }

    /// Query registered metadata without exposing granted dispatch.
    pub fn tool_spec(&self, path: &ToolPath) -> Result<&ToolSpec, LookupError> {
        self.tools.resolve(path).map(|tool| tool.spec())
    }

    /// Bind a registered tool to one recursive execution context.
    pub fn tool<'runtime, 'context>(
        &'runtime self,
        context: &'runtime C::Cx<'context>,
        path: ToolPath,
    ) -> Result<ToolInvocation<'runtime, 'context, C>, LookupError>
    where
        C: 'context,
        C::Cx<'context>: ToolInvocationContext,
    {
        let tool = self.tools.resolve(&path)?;
        let declared_capabilities: Capabilities =
            tool.spec().required_capabilities.iter().copied().collect();
        Ok(ToolInvocation::new(
            &self.kernel,
            tool,
            context,
            path,
            declared_capabilities,
        ))
    }
}

#[cfg(test)]
mod tests;
