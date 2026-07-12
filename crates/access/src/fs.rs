use std::path::{Path, PathBuf};

pub mod access;
pub mod action;
pub mod content_search;
pub mod copy;
pub mod directory;
pub mod error;
pub mod glob;
pub mod inspect;
pub mod path;
pub mod read_dir;
pub mod remove;
pub mod remove_dir;
pub mod rename;

pub use access::{FsAccess, FsAccessError, FsReadOutput, FsWriteOutput};
pub use action::{
    FsAction, FsAtomicWriteAction, FsContentSearchAction, FsContentSearchOptions, FsCopyFileAction,
    FsCreateDirAllAction, FsGlobAction, FsInspectPathAction, FsPathAction, FsReadAction,
    FsReadDirAction, FsRemoveDirAllAction, FsRemoveFileAction, FsRenameAction, FsResolvePathAction,
    FsWriteAction, FsWriteOptions,
};
pub use content_search::{FsContentSearchMatch, FsContentSearchOutput};
pub use copy::FsCopyFileOutput;
pub use directory::FsCreateDirAllOutput;
pub use error::FsActionError;
pub use glob::{FsGlobOutput, FsPathKind, FsPathMatch};
pub use inspect::FsInspectPathOutput;
pub use path::{GrantedEntryPath, GrantedPath, ResolvedEntryPath, ResolvedPath};
pub use read_dir::{FsReadDirEntry, FsReadDirOutput};
pub use remove::{FsRemoveFileKind, FsRemoveFileOutput};
pub use remove_dir::FsRemoveDirAllOutput;
pub use rename::FsRenameOutput;

/// Filesystem root view required by path resolution.
///
/// This lives with the fs access domain because resolving a relative path is
/// part of running `FsResolvePathAction`, not a core policy concept.
pub trait FsResolutionContext {
    fn fs_resolution_root(&self) -> &Path;
}

/// Filesystem root view required by fs path policy.
///
/// Allowed roots are policy input for `FsPathAction`; a granted
/// `FsResolvePathAction` prepares the resolved fact, while kernel policy decides
/// containment. Implementors should provide canonical/simplified roots in the
/// same path space as resolved action facts.
pub trait FsPathPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf];
}

#[cfg(test)]
mod tests;
