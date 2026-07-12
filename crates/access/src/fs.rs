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

pub use access::{FsAccess, FsAccessError, FsReadOutput, FsWriteOutput};
pub use action::{
    FsAction, FsAtomicWriteAction, FsContentSearchAction, FsContentSearchOptions, FsCopyFileAction,
    FsCreateDirAllAction, FsGlobAction, FsInspectPathAction, FsReadAction, FsReadDirAction,
    FsRemoveFileAction, FsResolvePathAction, FsWriteAction, FsWriteOptions,
};
pub use content_search::{FsContentSearchMatch, FsContentSearchOutput};
pub use copy::FsCopyFileOutput;
pub use directory::FsCreateDirAllOutput;
pub use error::FsActionError;
pub use glob::{FsGlobOutput, FsPathKind, FsPathMatch};
pub use inspect::FsInspectPathOutput;
pub use path::GrantedPath;
pub use read_dir::{FsReadDirEntry, FsReadDirOutput};
pub use remove::{FsRemoveFileKind, FsRemoveFileOutput};

/// Filesystem root view required by path resolution.
///
/// This lives with the fs access domain because resolving a relative path is
/// part of preparing `FsResolvePathAction` facts, not a core policy concept.
pub trait FsResolutionContext {
    fn fs_resolution_root(&self) -> &Path;
}

/// Filesystem root view required by fs path policy.
///
/// Allowed roots are policy input for `FsResolvePathAction`; access prepares the
/// resolved path fact, while kernel policy decides containment. Implementors
/// should provide canonical/simplified roots in the same path space as resolved
/// action facts.
pub trait FsPathPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf];
}

#[cfg(test)]
mod tests;
