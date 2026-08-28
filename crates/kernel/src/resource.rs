//! Typed singleton resources available to policy and tool contexts.

use std::path::PathBuf;

use anymap2::SendSyncAnyMap;

/// A singleton resource keyed by its Rust type.
///
/// Resources are operational facts such as a workspace root. They are not
/// capabilities: a resource helps an action describe itself, while the policy
/// engine still decides whether that action may run.
pub trait Resource: 'static + Send + Sync {
    /// Stable name used in error messages.
    const NAME: &'static str;
}

/// The workspace root available to filesystem tools.
#[derive(Debug, Clone)]
pub struct WorkspaceRoot(pub PathBuf);

impl Resource for WorkspaceRoot {
    const NAME: &'static str = "WorkspaceRoot";
}

/// Type-erased resource collection shared by one facade.
///
/// The builder owns this value during agent assembly and hands it to
/// [`Facade::with_resources`](crate::Facade::with_resources). From then on it
/// is shared immutably through facade clones, so prompt-loop snapshots see a
/// stable set while they run.
#[derive(Debug, Default)]
pub struct Resources {
    inner: SendSyncAnyMap,
}

impl Resources {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<R: Resource>(&mut self, value: R) -> Option<R> {
        self.inner.insert(value)
    }

    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<&R> {
        self.inner.get::<R>()
    }

    #[must_use]
    pub fn contains<R: Resource>(&self) -> bool {
        self.inner.contains::<R>()
    }
}
