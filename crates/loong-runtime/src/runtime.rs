//! Long-lived owner for governance and runtime registries.

use loong_contracts::{ToolPath, ToolSpec};
use loong_core::policy::context::ContextFactory;
use loong_kernel::{AccessCx, AuditError, AuditEventKind, Kernel};

use crate::tool_plane::{
    ToolInvocation, ToolInvocationContext, ToolPlaneRegistry, ToolRegistration, error::LookupError,
};

/// Process-local identity of one Runtime ownership domain.
///
/// This is neither authorization evidence nor a persisted session id. App
/// Session owners use it only to prevent borrowing a Session through a
/// different Runtime instance with another registry/policy domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeId(uuid::Uuid);

/// Owns the kernel and tool plane shared by app sessions and execution contexts.
///
/// This is the runtime authority root, not a kernel facade or UI state object.
/// The concrete context remains app-defined through `C`. Runtime owns the
/// concrete `ToolPlaneRegistry`; its storage strategy stays encapsulated inside
/// the registry, while registered dispatch remains sealed behind Runtime's
/// audited `ToolInvocation` path.
pub struct Runtime<C: ContextFactory> {
    id: RuntimeId,
    kernel: Kernel<C>,
    tools: ToolPlaneRegistry<C>,
}

impl<C> Runtime<C>
where
    C: ContextFactory,
{
    pub fn new(kernel: Kernel<C>, tools: ToolPlaneRegistry<C>) -> Self {
        Self {
            id: RuntimeId(uuid::Uuid::new_v4()),
            kernel,
            tools,
        }
    }

    #[must_use]
    pub fn id(&self) -> RuntimeId {
        self.id
    }

    /// Expose the concrete kernel only to named legacy bearer owners.
    ///
    /// The unmigrated memory plane and final ingress fallback still consume old
    /// pack/token envelopes. Typed Tool and Access paths must use this Runtime's
    /// dedicated methods; any other caller is a boundary violation that can be
    /// found mechanically. Delete this method after those legacy owners migrate.
    #[must_use]
    pub fn legacy_kernel(&self) -> &Kernel<C> {
        &self.kernel
    }

    /// Bind the Runtime's governance authority to one typed execution context.
    #[must_use]
    pub fn access<'runtime, 'context>(
        &'runtime self,
        context: &'runtime C::Cx<'context>,
    ) -> AccessCx<'runtime, 'context, C>
    where
        C: 'context,
    {
        AccessCx::new(&self.kernel, context)
    }

    /// Record app-owned operational evidence without exposing the concrete Kernel.
    ///
    /// Authorization and typed execution evidence remain reserved to their
    /// proof-owning runtime paths; Kernel rejects those event families here.
    pub fn record_audit_event(
        &self,
        actor_id: Option<&str>,
        kind: AuditEventKind,
    ) -> Result<(), AuditError> {
        self.kernel.record_audit_event(actor_id, kind)
    }

    #[must_use]
    pub fn registered_tool_paths(&self) -> Vec<ToolPath> {
        self.tools.registered_paths()
    }

    /// Query registration and concrete spec without exposing dispatch.
    pub fn tool_metadata(
        &self,
        path: &ToolPath,
    ) -> Result<(&ToolRegistration, &ToolSpec), LookupError> {
        self.tools
            .resolve(path)
            .map(|(_canonical_path, tool)| (tool.registration(), tool.spec()))
    }

    /// Bind a registered tool to one recursive execution context.
    pub fn tool<'runtime, 'context>(
        &'runtime self,
        context: &C::Cx<'context>,
        path: ToolPath,
    ) -> Result<ToolInvocation<'runtime, 'context, C>, LookupError>
    where
        C: 'context,
        C::Cx<'context>: ToolInvocationContext,
    {
        let (canonical_path, tool) = self.tools.resolve(&path)?;
        Ok(ToolInvocation::new(
            &self.kernel,
            tool,
            context.clone(),
            canonical_path.clone(),
        ))
    }
}

#[cfg(test)]
mod tests;
