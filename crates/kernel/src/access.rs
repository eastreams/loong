use loong_access::fs::{FsAccess, FsAccessError, FsActionError};
use loong_core::{kernel::Kernel as CoreKernel, policy::context::ContextFactory};

use crate::{kernel::Kernel, policy::PolicyPipeline};

/// Kernel-defined access facade.
///
/// Keep this type small: it owns no policy logic and no backend state. It only
/// carries the kernel reference plus the action context into concrete access
/// modules such as `loong_access::fs`.
pub struct AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
{
    kernel: &'a Kernel<C>,
    policy_context: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C> AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
{
    #[inline(always)]
    #[must_use]
    pub fn new(kernel: &'a Kernel<C>, policy_context: &'a C::Cx<'ctx>) -> Self {
        Self {
            kernel,
            policy_context,
        }
    }

    /// Filesystem access entry point.
    ///
    /// Callers should prefer `ctx.access().fs().read_file(path)` over direct
    /// filesystem I/O. The concrete `FsAccess` will resolve paths, request a
    /// typed action grant, and perform the read.
    #[inline(always)]
    #[must_use]
    pub fn fs(self) -> FsAccess<'a, 'ctx, C, PolicyPipeline<C>> {
        FsAccess::new(self.kernel.policy_engine(), self.policy_context)
    }
}

/// Context capability for tools that need governed access facades.
///
/// Concrete tool implementations live outside `loong-app`, so they cannot rely
/// on `AppExecutionContext` directly. This trait is the narrow boundary they
/// need: given the current invocation context, obtain the kernel-defined access
/// facade and let access/actions perform policy-gated side effects.
pub trait KernelAccess<C>
where
    C: ContextFactory,
{
    fn access(&self) -> AccessCx<'_, '_, C>;
}

#[must_use]
pub fn fs_read_error_is_policy_denial(error: &FsAccessError) -> bool {
    matches!(
        error,
        FsAccessError::Authorization(_)
            | FsAccessError::Action(FsActionError::PathEscapesAllowedRoots { .. })
    )
}

#[cfg(test)]
mod tests;
