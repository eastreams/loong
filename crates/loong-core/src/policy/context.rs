use std::collections::BTreeSet;

use loong_contracts::Capability;

/// Policy-facing authority available for every governed action.
///
/// Keep this view independent of app concrete context types. Action-specific
/// policies add narrower requirement traits beside the action they govern.
pub trait PolicyContext: Send + Sync {
    /// Capabilities currently available to the recursive execution scope.
    fn allowed_capabilities(&self) -> &BTreeSet<Capability>;
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
