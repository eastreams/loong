use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use loong_contracts::Capability;
use loong_core::policy::action::{ActionMeta, ActionMetadata};
use serde_json::{Value, json};

use super::path::{GrantedPath, ResolvedPath};

const FS_RESOLVE_REQUIRED_CAPABILITIES: [Capability; 0] = [];
const FS_READ_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

/// Typed action for resolving a raw path into a governed fs path.
///
/// Path resolution observes the filesystem through canonicalization and symlink
/// handling, so it is modeled as an action instead of an ungoverned helper.
/// The action carries resolved path facts for policy; it does not decide
/// whether those facts are allowed.
/// It does not declare read/search/glob capabilities; concrete data-leaking fs
/// actions declare those after receiving the governed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsResolvePathAction {
    raw_path: PathBuf,
    resolved: ResolvedPath,
}

impl FsResolvePathAction {
    // Access prepares filesystem-observation facts before policy evaluation.
    // The constructor stays fs-private so callers cannot mint a resolve action
    // without using the context roots from `FsAccess`.
    pub(in crate::fs) fn resolve(
        path: impl AsRef<Path>,
        resolution_root: impl AsRef<Path>,
        allowed_roots: &[PathBuf],
    ) -> Result<Self, super::error::FsActionError> {
        let raw_path = path.as_ref().to_path_buf();
        let resolved = ResolvedPath::resolve(&raw_path, resolution_root, allowed_roots)?;
        Ok(Self { raw_path, resolved })
    }

    #[must_use]
    pub fn raw_path(&self) -> &Path {
        &self.raw_path
    }

    #[must_use]
    pub fn resolved_path(&self) -> &Path {
        self.resolved.path()
    }

    #[must_use]
    pub fn allowed_roots(&self) -> &[PathBuf] {
        self.resolved.allowed_roots()
    }

    pub(in crate::fs) fn into_granted_path(self) -> GrantedPath {
        GrantedPath::new(self.resolved.into_path_buf())
    }
}

impl ActionMeta for FsResolvePathAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.resolve_path",
            operation: Cow::Borrowed("resolve_path"),
            required_capabilities: Cow::Borrowed(&FS_RESOLVE_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.raw_path.display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.raw_path.display().to_string(),
            "resolved_path": self.resolved_path().display().to_string(),
            "allowed_roots": self
                .allowed_roots()
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>(),
        }))
    }
}

/// Typed action for reading one canonical filesystem path.
///
/// The action declares the required capability and audit resource. It does not
/// carry workspace roots; roots belong to the invocation context and path
/// resolver. Its constructor accepts `GrantedPath` so path policy cannot be
/// bypassed with a raw or merely canonical `PathBuf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadAction {
    path: GrantedPath,
}

impl FsReadAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsReadAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.read",
            operation: Cow::Borrowed("read_file"),
            required_capabilities: Cow::Borrowed(&FS_READ_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Filesystem action family.
///
/// Keep variants here thin wrappers around typed actions so policies can
/// register either for a concrete action or for the family as needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsAction {
    Read(FsReadAction),
}

impl FsAction {
    #[must_use]
    pub fn read_file(path: GrantedPath) -> Self {
        Self::Read(FsReadAction::new(path))
    }
}

impl ActionMeta for FsAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        match self {
            Self::Read(action) => action.metadata(),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Read(action) => action.audit_resource(),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        match self {
            Self::Read(action) => action.payload(),
        }
    }
}
