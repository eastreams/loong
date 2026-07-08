use std::{collections::BTreeSet, path::Path};

use loong_contracts::{Capability, ExecutionPlane, PlaneTier};

pub trait PolicyContext: Send + Sync {
    /// Allowed capabilities
    fn capabilities(&self) -> BTreeSet<Capability>;
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

pub trait ActionContext: PolicyContext {
    fn execution_plane(&self) -> ExecutionPlane;

    fn plane_tier(&self) -> PlaneTier;
}

pub trait WorkspacePolicyContext: PolicyContext {
    fn workspace_root(&self) -> &Path;
}
