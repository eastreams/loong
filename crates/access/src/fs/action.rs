use std::{borrow::Cow, collections::BTreeSet, path::Path};

use loong_contracts::Capability;
use loong_core::policy::action::Action;

use super::path::CanonicalPath;

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

impl Action for FsReadAction {
    fn kind(&self) -> &'static str {
        "fs.read"
    }

    fn operation(&self) -> Cow<'static, str> {
        Cow::Borrowed("read_file")
    }

    fn audit_resource(&self) -> Option<Cow<'static, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn required_capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::FilesystemRead])
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

impl Action for FsAction {
    fn kind(&self) -> &'static str {
        match self {
            Self::Read(action) => action.kind(),
        }
    }

    fn operation(&self) -> Cow<'static, str> {
        match self {
            Self::Read(action) => action.operation(),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'static, str>> {
        match self {
            Self::Read(action) => action.audit_resource(),
        }
    }

    fn required_capabilities(&self) -> BTreeSet<Capability> {
        match self {
            Self::Read(action) => action.required_capabilities(),
        }
    }
}
