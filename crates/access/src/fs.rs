use loong_core::policy::PolicyEngine;

mod read;
pub use read::FsReadError;

/// Filesystem access facade.
///
/// This module is the side-effect boundary for governed filesystem operations.
/// Callers provide raw paths and operation inputs; `FsAccess` resolves paths,
/// builds typed actions, asks policy for grants, and only then touches disk.
#[expect(
    dead_code,
    reason = "facade state remains unused until resolved-path operations are exposed"
)]
pub struct FsAccess<'a, Cx: Sync, P>
where
    P: PolicyEngine<Cx>,
    // Cx: FsResolutionContext + FsPathPolicyContext,
{
    pub(in crate::fs) policy_engine: &'a P,
    pub(in crate::fs) ctx: &'a Cx,
}

impl<'a, Cx: Sync, P> FsAccess<'a, Cx, P>
where
    P: PolicyEngine<Cx>,
    // Cx: FsResolutionContext + FsPathPolicyContext,
{
    #[inline(always)]
    #[must_use]
    pub fn new(policy_engine: &'a P, ctx: &'a Cx) -> Self {
        Self { policy_engine, ctx }
    }
}
