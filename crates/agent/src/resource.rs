//! Tool-set resource declarations.

pub use kernel::resource::{Resource, Resources, WorkspaceRoot};

/// A tool set's declared dependency on one resource type.
#[derive(Debug, Clone, Copy)]
pub struct ResourceNeed {
    name: &'static str,
    check: fn(&Resources) -> bool,
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

    pub(crate) fn is_satisfied_by(&self, resources: &Resources) -> bool {
        (self.check)(resources)
    }
}
