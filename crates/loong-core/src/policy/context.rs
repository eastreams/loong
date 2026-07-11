use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loong_contracts::Capability;

pub trait PolicyContext: Send + Sync {
    /// Allowed capabilities
    fn allowed_capabilities(&self) -> BTreeSet<Capability>;
}

/// Filesystem view required by fs access.
///
/// This lives in core so app-defined contexts can implement it without
/// depending on `loong-access`; access modules consume the trait through the
/// shared policy/access context boundary.
pub trait FsAccessContext {
    fn fs_resolution_root(&self) -> &Path;

    fn fs_allowed_roots(&self) -> &[PathBuf];
}

/// Type-level factory for policy/access execution contexts.
///
/// `ContextFactory` maps a borrow lifetime to the concrete context type used
/// by one runtime integration. It does not construct context values; app or
/// runtime code owns value construction and passes contexts into kernel calls.
pub trait ContextFactory: Send + Sync + 'static {
    type Cx<'a>: PolicyContext
    where
        Self: 'a;
}
