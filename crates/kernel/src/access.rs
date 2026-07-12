use loong_access::fs::FsAccess;
use loong_core::{kernel::Kernel as CoreKernel, policy::context::ContextFactory};

use crate::{kernel::Kernel, policy::PolicyPipeline};

pub mod fs {
    pub use loong_access::fs::{
        FsAccess, FsAccessError, FsAtomicWriteAction, FsContentSearchAction, FsContentSearchMatch,
        FsContentSearchOptions, FsContentSearchOutput, FsCopyFileAction, FsCopyFileOutput,
        FsCreateDirAllAction, FsCreateDirAllOutput, FsGlobAction, FsGlobOutput,
        FsInspectPathAction, FsInspectPathOutput, FsPathKind, FsPathMatch, FsPathPolicyContext,
        FsReadAction, FsReadDirAction, FsReadDirEntry, FsReadDirOutput, FsReadOutput,
        FsRemoveDirAllAction, FsRemoveDirAllOutput, FsRemoveFileAction, FsRemoveFileKind,
        FsRemoveFileOutput, FsRenameAction, FsRenameOutput, FsResolutionContext,
        FsResolvePathAction, FsWriteAction, FsWriteOptions, FsWriteOutput,
    };
}

use fs::FsResolutionContext;

/// Kernel-defined access facade.
///
/// Keep this type small: it owns no policy logic and no backend state. It only
/// carries the kernel reference plus the action context into concrete access
/// modules exposed through `loong_kernel::access::fs`.
pub struct AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
{
    kernel: &'a Kernel<C>,
    ctx: &'a C::Cx<'ctx>,
}

impl<'a, 'ctx, C> AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
{
    #[inline(always)]
    #[must_use]
    pub fn new(kernel: &'a Kernel<C>, ctx: &'a C::Cx<'ctx>) -> Self {
        Self { kernel, ctx }
    }
}

impl<'a, 'ctx, C> AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
    C::Cx<'ctx>: FsResolutionContext,
{
    /// Filesystem access entry point.
    ///
    /// Callers should prefer `ctx.access().fs().read_file(path)` over direct
    /// filesystem I/O. The concrete `FsAccess` will resolve paths, request
    /// typed action grants, and perform the fs side effect.
    #[inline(always)]
    #[must_use]
    pub fn fs(self) -> FsAccess<'a, 'ctx, C, PolicyPipeline<C>> {
        FsAccess::new(self.kernel.policy_engine(), self.ctx)
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
    Self: FsResolutionContext,
{
    fn access(&self) -> AccessCx<'_, '_, C>;
}

#[cfg(test)]
mod tests;
