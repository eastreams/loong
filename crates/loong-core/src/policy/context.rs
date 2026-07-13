use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{Capability, PermissionResolution, PolicyReport};

use crate::{error::PermissionRequestError, policy::action::ActionMeta};

/// Policy-facing authority available for every governed action.
///
/// Keep this view independent of app concrete context types. Action-specific
/// policies add narrower requirement traits beside the action they govern.
#[async_trait]
pub trait PolicyContext: Send + Sync {
    /// Capabilities currently available to the recursive execution scope.
    fn allowed_capabilities(&self) -> &BTreeSet<Capability>;

    /// Request consent from the current session's parent.
    ///
    /// The default is intentionally a loud test/fixture failure. Production
    /// contexts must override this before any installed policy can return
    /// `RequireParentPermission`; an unavailable production interaction surface
    /// must return [`PermissionRequestError::Unavailable`] instead.
    #[expect(
        clippy::panic,
        reason = "default hook exposes a miswired permission policy in tests and fixtures"
    )]
    async fn request_parent_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        panic!("parent permission requested from a context without permission support")
    }

    /// Request consent from the user, the root authority outside the session tree.
    ///
    /// As with parent permission, production contexts must return a structured
    /// unavailable error rather than reaching this default.
    #[expect(
        clippy::panic,
        reason = "default hook exposes a miswired permission policy in tests and fixtures"
    )]
    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        panic!("user permission requested from a context without permission support")
    }
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
