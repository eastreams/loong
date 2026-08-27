//! Typed singleton resources that tool sets can require.

use std::path::PathBuf;

use anymap2::AnyMap;

/// A singleton resource provided to tools, keyed by its Rust type.
pub trait Resource: 'static + Send + Sync {
    /// Stable name used in error messages.
    const NAME: &'static str;
}

/// The workspace root available to filesystem tools.
pub struct WorkspaceRoot(pub PathBuf);

impl Resource for WorkspaceRoot {
    const NAME: &'static str = "WorkspaceRoot";
}

/// A tool set's declared dependency on one resource type.
#[derive(Debug, Clone, Copy)]
pub struct ResourceNeed {
    name: &'static str,
    check: fn(&AnyMap) -> bool,
}

impl ResourceNeed {
    #[must_use]
    pub fn of<R: Resource>() -> Self {
        Self {
            name: R::NAME,
            check: |resources| resources.contains::<R>(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub(crate) fn is_satisfied_by(&self, resources: &AnyMap) -> bool {
        (self.check)(resources)
    }
}
