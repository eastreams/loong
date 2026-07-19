use std::borrow::Cow;

use async_trait::async_trait;
use loong_contracts::{PolicyDecision, PolicyGrant, ToolPath, ToolPathError};
use loong_core::policy::{context::ContextFactory, policy::Policy};
use loong_runtime::tool_plane::{
    ToolInvocationAction, ToolPlaneRegistry, error::RegistrationError,
};
use thiserror::Error;

use crate::context::AppContextFactory;

/// Preserves the failing stage while app bootstrap composes path construction
/// with runtime registry insertion.
#[derive(Debug, Error)]
pub(crate) enum BuiltinToolPlaneError {
    #[error("invalid builtin tool path: {0}")]
    InvalidPath(#[from] ToolPathError),
    #[error(transparent)]
    Registration(#[from] RegistrationError),
}

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
            predicate: Some("tool invocation passed capability gate".into()),
            reason: "tool invocation allowed by app policy".into(),
        }
    }
}

/// Build the concrete builtin registry before it is erased into `Runtime<C>`.
///
/// Registration is deliberately fallible: a duplicate builtin path is a
/// bootstrap error, not a process-global initialization panic.
pub(crate) fn builtin_tool_plane()
-> Result<ToolPlaneRegistry<AppContextFactory>, BuiltinToolPlaneError> {
    let plane = ToolPlaneRegistry::new();
    #[cfg(feature = "tool-file")]
    let plane = {
        let mut plane = plane;
        // `read` is the aggregate typed facade; provider aliases such as
        // `file.read` canonicalize to this path before plane lookup.
        plane.register(
            ToolPath::new(["read"])?,
            loong_tools::file::ReadTool::new("read"),
        )?;

        plane.register(
            ToolPath::new(["write"])?,
            loong_tools::file::WriteTool::new("write"),
        )?;

        plane.register_with_success_observer(
            ToolPath::new(["edit"])?,
            loong_tools::file::EditTool::new("edit"),
            |_ctx, output: &loong_tools::file::EditOutput| {
                // Preview events are an app-runtime side channel; the
                // concrete tool only returns typed before/after data.
                crate::tools::file::emit_file_change_preview(
                    output.path.as_path(),
                    crate::tools::runtime_events::ToolFileChangeKind::Edit,
                    Some(output.before.as_str()),
                    output.after.as_str(),
                );
            },
        )?;

        // Legacy read-family discoverable paths keep their own typed entries so
        // audit path and response metadata do not collapse into `read` while fs
        // side effects continue moving to access actions.
        plane.register(
            ToolPath::new(["glob.search"])?,
            loong_tools::file::GlobSearchTool::new("glob.search"),
        )?;

        plane.register(
            ToolPath::new(["content.search"])?,
            loong_tools::file::ContentSearchTool::new("content.search"),
        )?;
        plane
    };

    Ok(plane)
}

/// Build the invariant builtin registry for tests without repeating lint
/// exceptions across fixtures. Production bootstrap must use the fallible API.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn test_builtin_tool_plane() -> ToolPlaneRegistry<AppContextFactory> {
    builtin_tool_plane().expect("builtin tool registration should succeed")
}

#[cfg(test)]
mod tests;
