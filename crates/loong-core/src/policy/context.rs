use std::borrow::Cow;

use async_trait::async_trait;
use loong_contracts::{AuthorizationSubject, Capabilities, PermissionResolution, PolicyReport};

use crate::{error::PermissionRequestError, policy::action::ActionMeta};

/// Policy-facing authority available for every governed action.
///
/// Keep this view independent of app concrete context types. Action-specific
/// policies add narrower requirement traits beside the action they govern.
#[async_trait]
pub trait PolicyContext: Send + Sync {
    /// Capabilities currently available to the recursive execution scope.
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities>;

    /// Stable actor and typed-or-legacy authority scope for authorization evidence.
    fn authorization_subject(&self) -> AuthorizationSubject;

    /// Request consent from the current session's parent.
    ///
    /// Contexts without a parent permission interaction fail closed with
    /// [`PermissionRequestError::Unavailable`] by default.
    async fn request_parent_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        Err(PermissionRequestError::Unavailable {
            reason: Cow::Borrowed("parent permission interaction is unavailable"),
        })
    }

    /// Request consent from the user, the root authority outside the session tree.
    ///
    /// Contexts without a user permission interaction fail closed with
    /// [`PermissionRequestError::Unavailable`] by default.
    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        Err(PermissionRequestError::Unavailable {
            reason: Cow::Borrowed("user permission interaction is unavailable"),
        })
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
