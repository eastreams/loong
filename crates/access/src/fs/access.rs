use loong_core::policy::{context::ContextFactory, engine::PolicyEngine};

use super::path::{FsPathPolicyContext, FsResolutionContext};

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
/// Callers provide raw paths and operation inputs; `FsAccess` resolves paths,
/// builds typed actions, asks policy for grants, and only then touches disk.
pub struct FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    pub(in crate::fs) policy_engine: &'a P,
    pub(in crate::fs) ctx: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    #[inline(always)]
    #[must_use]
    pub fn new(policy_engine: &'a P, ctx: &'a C::Cx<'ctx>) -> Self {
        Self { policy_engine, ctx }
    }
}
