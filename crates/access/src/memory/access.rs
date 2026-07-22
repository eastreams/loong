use std::path::Path;

use loong_core::policy::{context::ContextFactory, engine::PolicyEngine};

/// Narrow Context view used to bind memory operations to the current Session.
///
/// Callers do not pass arbitrary session ids to MemoryAccess. The app-defined
/// recursive Context supplies the target, so child execution cannot use a
/// parent or sibling memory namespace by changing an action argument.
pub trait MemorySessionContext {
    fn memory_session_id(&self) -> &str;
}

/// Stable workspace projection captured into composite memory Actions.
///
/// The Access facade reads this value from Context; callers never provide a
/// root argument beside the Action request.
pub trait MemoryWorkspaceContext {
    fn memory_workspace_root(&self) -> Option<&Path>;
}

/// Governed memory facade for one recursive execution Context.
pub struct MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
{
    pub(super) policy_engine: &'a P,
    pub(super) ctx: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
{
    #[must_use]
    pub fn new(policy_engine: &'a P, ctx: &'a C::Cx<'ctx>) -> Self {
        Self { policy_engine, ctx }
    }
}
