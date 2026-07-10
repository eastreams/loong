use std::{borrow::Cow, path::Path};

use loong_contracts::Capability;
use loong_core::policy::action::{ActionMeta, ActionMetadata};
use serde_json::{Value, json};

use super::path::CanonicalPath;

const FS_READ_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

/// Typed action for reading one canonical filesystem path.
///
/// The action declares the required capability and audit resource. It does not
/// carry workspace roots; roots belong to the invocation context and path
/// resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadAction {
    path: CanonicalPath,
}

impl FsReadAction {
    #[must_use]
    pub fn new(path: CanonicalPath) -> Self {
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
    pub fn read_file(path: CanonicalPath) -> Self {
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
