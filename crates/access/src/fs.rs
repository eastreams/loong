use std::path::{Path, PathBuf};

pub mod access;
pub mod action;
pub mod error;
pub mod path;

pub use access::{FsAccess, FsAccessError, FsGlobOutput, FsPathKind, FsPathMatch, FsReadOutput};
pub use action::{FsAction, FsGlobAction, FsReadAction, FsResolvePathAction};
pub use error::FsActionError;
pub use path::GrantedPath;

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
