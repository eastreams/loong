use std::{any::Any, borrow::Cow};

use async_trait::async_trait;
use loong_contracts::{PolicyDecision, PolicyGrant, ToolPath, ToolPathError};
use loong_core::policy::{
    action::ActionMeta,
    context::ContextFactory,
    policy::{Policy, PolicyAny},
};
use loong_runtime::tool_plane::{
    ToolInvocationAction, ToolPlaneRegistry, error::RegistrationError,
};
use thiserror::Error;

use crate::context::RuntimeContextFactory;

pub(crate) mod consent;
#[cfg(feature = "tool-file")]
mod file;

/// Narrow app authority required by the broad tool-invocation policy.
///
/// The policy remains generic over ContextFactory and cannot depend on the
/// app's concrete recursive Context storage.
pub(crate) trait ToolVisibilityContext {
    fn tool_is_visible(&self, path: &ToolPath) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolVisibilityPolicy;

#[async_trait]
impl<C> PolicyAny<C> for ToolVisibilityPolicy
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: ToolVisibilityContext,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tool-visibility")
    }

    async fn grant(&self, ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyGrant {
        // Metadata is diagnostic, not authority. Only the runtime-owned common
        // Tool action may enter this broad policy; another Action cannot obtain
        // a grant by copying the `tool.invoke` strings.
        let Some(action) = (action as &dyn Any).downcast_ref::<ToolInvocationAction>() else {
            return PolicyGrant {
                decision: PolicyDecision::Continue,
                predicate: Some("action is not a tool invocation".into()),
                reason: "tool visibility policy does not apply".into(),
            };
        };

        let path = action.path();
        if ctx.tool_is_visible(path) {
            PolicyGrant {
                // Visibility is a hard gate, not sufficient authorization.
                // Typed action policy must still decide whether execution is allowed.
                decision: PolicyDecision::Continue,
                predicate: Some("tool path is visible in the recursive execution scope".into()),
                reason: "tool visibility gate passed".into(),
            }
        } else {
            PolicyGrant {
                decision: PolicyDecision::Deny,
                predicate: Some("tool path is hidden from the recursive execution scope".into()),
                reason: format!("tool `{path}` is not visible").into(),
            }
        }
    }
}

/// Terminal allow for the runtime's common tool-dispatch action.
///
/// Broad pre-policies enforce cross-tool constraints first. Concrete tools must
/// not add per-tool invocation policy here; their governed side effects belong
/// to typed Access actions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolInvocationAllowPolicy;

#[async_trait]
impl<C> Policy<C, ToolInvocationAction> for ToolInvocationAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tool-invocation-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &ToolInvocationAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("tool invocation passed broad runtime policy".into()),
            reason: "tool invocation allowed after runtime gates".into(),
        }
    }
}

/// Preserves whether builtin registration failed while constructing identity
/// or while inserting the concrete tool into the runtime directory.
#[derive(Debug, Error)]
pub(crate) enum BuiltinToolPlaneError {
    #[error("invalid builtin tool path: {0}")]
    InvalidPath(#[from] ToolPathError),
    #[error(transparent)]
    Registration(#[from] RegistrationError),
}

/// Build the concrete builtin registry before it is erased into `Runtime<C>`.
///
/// Registration is deliberately fallible: a duplicate builtin path is a
/// bootstrap error, not a process-global initialization panic.
pub(crate) fn builtin_tool_plane()
-> Result<ToolPlaneRegistry<RuntimeContextFactory>, BuiltinToolPlaneError> {
    let plane = ToolPlaneRegistry::new();
    #[cfg(feature = "tool-file")]
    let plane = file::register(plane)?;

    Ok(plane)
}

/// Build the invariant builtin registry for tests without repeating lint
/// exceptions across fixtures. Production bootstrap must use the fallible API.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn test_builtin_tool_plane() -> ToolPlaneRegistry<RuntimeContextFactory> {
    builtin_tool_plane().expect("builtin tool registration should succeed")
}

#[cfg(test)]
mod tests;
