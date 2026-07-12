//! Long-lived owner for governance and runtime registries.

use loong_core::policy::context::ContextFactory;
use loong_kernel::Kernel;

use crate::tool_plane::{ToolInvocationAction, ToolPath, ToolPlane};

/// Owns the kernel and tool plane shared by app sessions and execution contexts.
///
/// This is the runtime authority root, not a kernel facade or UI state object.
/// The concrete context remains app-defined through `C`, while the erased plane
/// permits replacing registry storage without propagating another generic.
pub struct Runtime<C: ContextFactory> {
    kernel: Kernel<C>,
    tools:
        Box<dyn ToolPlane<C, Path = ToolPath, InvocationAction = ToolInvocationAction> + 'static>,
}

impl<C> Runtime<C>
where
    C: ContextFactory,
{
    pub fn new<P>(kernel: Kernel<C>, tools: P) -> Self
    where
        P: ToolPlane<C, Path = ToolPath, InvocationAction = ToolInvocationAction> + 'static,
    {
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
    pub fn tools(
        &self,
    ) -> &(dyn ToolPlane<C, Path = ToolPath, InvocationAction = ToolInvocationAction> + 'static)
    {
        self.tools.as_ref()
    }
}

#[cfg(test)]
mod tests;
