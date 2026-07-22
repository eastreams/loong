use loong_access::fs::FsAccess;
use loong_access::memory::MemoryAccess;
use loong_core::{
    kernel::Kernel as CoreKernel,
    policy::{context::ContextFactory, engine::PolicyEngine},
};

use crate::kernel::Kernel;

pub mod fs {
    pub use loong_access::fs::{
        FsAccess, FsAtomicWriteAction, FsAtomicWriteAllowPolicy, FsContentSearchAction,
        FsContentSearchAllowPolicy, FsContentSearchError, FsContentSearchMatch,
        FsContentSearchOptions, FsContentSearchOutput, FsCopyFileAction, FsCopyFileAllowPolicy,
        FsCopyFileError, FsCopyFileOutput, FsCreateDirAllAction, FsCreateDirAllAllowPolicy,
        FsCreateDirAllError, FsCreateDirAllOutput, FsGlobAction, FsGlobAllowPolicy, FsGlobError,
        FsGlobOutput, FsInspectPathAction, FsInspectPathAllowPolicy, FsInspectPathError,
        FsInspectPathOutput, FsPathAction, FsPathAllowedRootsPolicy, FsPathError, FsPathKind,
        FsPathMatch, FsPathPolicyContext, FsReadAction, FsReadAllowPolicy, FsReadDirAction,
        FsReadDirAllowPolicy, FsReadDirEntry, FsReadDirError, FsReadDirOutput, FsReadError,
        FsReadFilenameDenyPolicy, FsReadOutput, FsRemoveDirAllAction, FsRemoveDirAllAllowPolicy,
        FsRemoveDirAllError, FsRemoveDirAllOutput, FsRemoveFileAction, FsRemoveFileAllowPolicy,
        FsRemoveFileError, FsRemoveFileKind, FsRemoveFileOutput, FsRenameAction,
        FsRenameAllowPolicy, FsRenameError, FsRenameOutput, FsResolutionContext,
        FsResolvePathAction, FsResolvePathAllowPolicy, FsWriteAction, FsWriteAllowPolicy,
        FsWriteError, FsWriteOptions, FsWriteOutput, GrantedEntryPath, GrantedPath,
        ResolvedEntryPath, ResolvedPath, normalize_path_lexically,
    };
}

pub mod memory {
    pub use loong_access::memory::{
        MemoryAccess, MemoryAccessError, MemoryAppendTurnAction, MemoryAppendTurnAllowPolicy,
        MemoryBackend, MemoryBackendError, MemoryCompactAction, MemoryCompactAllowPolicy,
        MemoryExecutionContext, MemoryReadStageEnvelopeAction, MemoryReadStageEnvelopeAllowPolicy,
        MemoryReplaceTurnsAction, MemoryReplaceTurnsAllowPolicy, MemoryReplaceTurnsOutcome,
        MemorySessionContext, MemorySnapshot, MemoryTranscriptAction, MemoryTranscriptAllowPolicy,
        MemoryTurn, MemoryWindowAction, MemoryWindowAllowPolicy, MemoryWorkspaceContext,
    };
}

use fs::{FsPathPolicyContext, FsResolutionContext};

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

    /// Memory access bound to the current recursive Context's Session.
    pub fn memory(self) -> MemoryAccess<'a, 'ctx, C, impl PolicyEngine<C> + 'a> {
        MemoryAccess::new(self.kernel.policy_engine(), self.ctx)
    }
}

impl<'a, 'ctx, C> AccessCx<'a, 'ctx, C>
where
    C: ContextFactory + 'ctx,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Filesystem access entry point.
    ///
    /// Callers should prefer `ctx.access().fs().read_file(path)` over direct
    /// filesystem I/O. The concrete `FsAccess` will resolve paths, request
    /// typed action grants, and perform the fs side effect.
    #[inline(always)]
    #[must_use]
    pub fn fs(self) -> FsAccess<'a, 'ctx, C, impl PolicyEngine<C> + 'a> {
        FsAccess::new(self.kernel.policy_engine(), self.ctx)
    }
}

/// Context capability for tools that need governed access facades.
///
/// Concrete tool implementations live outside `loong-app`, so they cannot rely
/// on its concrete `Context`. This trait is the narrow boundary they need:
/// given the current invocation context, obtain the kernel-defined access facade
/// and let access/actions perform policy-gated side effects.
pub trait KernelAccess<C>
where
    C: ContextFactory,
{
    fn access(&self) -> AccessCx<'_, '_, C>;
}

#[cfg(test)]
mod tests;
