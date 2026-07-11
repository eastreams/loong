use std::collections::BTreeSet;

use loong_contracts::Capability;

pub trait CapabilityContext: Send + Sync {
    /// Allowed capabilities
    fn allowed_capabilities(&self) -> BTreeSet<Capability>;
}

/// Type-level factory for policy/access execution contexts.
///
/// `ContextFactory` maps a borrow lifetime to the concrete context type used
/// by one runtime integration. It does not construct context values; app or
/// runtime code owns value construction and passes contexts into kernel calls.
pub trait ContextFactory: Send + Sync + 'static {
    type Cx<'a>: CapabilityContext
    where
        Self: 'a;
}
