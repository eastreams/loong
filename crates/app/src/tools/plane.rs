use std::{borrow::Cow, sync::OnceLock};

use async_trait::async_trait;
use loong_contracts::{PolicyDecision, PolicyGrant, ToolExecutionError};
use loong_core::{
    policy::{context::ContextFactory, policy::Policy},
    tool::ToolProvenance,
};
use loong_runtime::tool_plane::{ToolInvocationAction, ToolPath, ToolPlane, ToolPlaneRegistry};

use crate::context::AppContextFactory;

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

// TODO(runtime-owner): remove this lint exception with the global OnceLock.
// `Runtime<C>` must construct the builtin plane through a fallible bootstrap,
// so duplicate registrations are returned to the host instead of panicking.
#[allow(clippy::expect_used)]
pub(crate) fn app_tool_plane()
-> &'static dyn ToolPlane<AppContextFactory, Path = ToolPath, InvocationAction = ToolInvocationAction>
{
    static TOOL_PLANE: OnceLock<ToolPlaneRegistry<AppContextFactory>> = OnceLock::new();
    TOOL_PLANE.get_or_init(|| {
        #[allow(unused_mut)]
        let mut plane = ToolPlaneRegistry::new();
        #[cfg(feature = "tool-file")]
        // `read` is the aggregate typed facade; provider aliases such as
        // `file.read` canonicalize to this path before plane lookup.
        {
            plane
                .register_with_provenance(
                    ToolPath::from("read"),
                    ToolProvenance::Builtin,
                    loong_tools::file::ReadTool::new("read"),
                )
                .expect("builtin typed tool path `read` must be unique");

            plane
                .register_with_provenance(
                    ToolPath::from("write"),
                    ToolProvenance::Builtin,
                    loong_tools::file::WriteTool::new("write"),
                )
                .expect("builtin typed tool path `write` must be unique");

            plane
                .register_with_provenance_and_success_observer(
                    ToolPath::from("edit"),
                    ToolProvenance::Builtin,
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
                        Ok::<(), ToolExecutionError>(())
                    },
                )
                .expect("builtin typed tool path `edit` must be unique");

            // Legacy read-family discoverable paths keep their own typed
            // entries so audit path and response metadata do not collapse into
            // the aggregate `read` surface while fs side effects still move to
            // access actions.
            plane
                .register_with_provenance(
                    ToolPath::from("glob.search"),
                    ToolProvenance::Builtin,
                    loong_tools::file::GlobSearchTool::new("glob.search"),
                )
                .expect("builtin typed tool path `glob.search` must be unique");

            plane
                .register_with_provenance(
                    ToolPath::from("content.search"),
                    ToolProvenance::Builtin,
                    loong_tools::file::ContentSearchTool::new("content.search"),
                )
                .expect("builtin typed tool path `content.search` must be unique");
        }
        plane
    })
}

#[cfg(test)]
mod tests;
