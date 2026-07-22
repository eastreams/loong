//! File tool registrations compiled only with the complete file surface.

use loong_contracts::ToolPath;
use loong_runtime::tool_plane::{ToolPlaneRegistry, ToolRegistration};

use super::BuiltinToolPlaneError;
use crate::context::RuntimeContextFactory;

pub(super) fn register(
    mut plane: ToolPlaneRegistry<RuntimeContextFactory>,
) -> Result<ToolPlaneRegistry<RuntimeContextFactory>, BuiltinToolPlaneError> {
    plane.register(
        ToolPath::new(["read"])?,
        ToolRegistration::direct("read"),
        loong_tools::file::ReadTool,
    )?;
    plane.register(
        ToolPath::new(["write"])?,
        ToolRegistration::direct("write"),
        loong_tools::file::WriteTool,
    )?;
    plane.register_with_success_observer(
        ToolPath::new(["edit"])?,
        ToolRegistration::direct("edit"),
        loong_tools::file::EditTool,
        |_ctx, output: &loong_tools::file::EditOutput| {
            // Preview events are an app-runtime side channel; the concrete
            // tool only returns typed before/after data.
            crate::tools::file::emit_file_change_preview(
                output.path.as_path(),
                crate::tools::runtime_events::ToolFileChangeKind::Edit,
                Some(output.before.as_str()),
                output.after.as_str(),
            );
        },
    )?;

    // Search operations keep distinct paths for discovery and audit while the
    // aggregate `read` tool may dispatch to the same concrete implementations.
    plane.register(
        ToolPath::new(["glob.search"])?,
        ToolRegistration::discoverable("glob.search"),
        loong_tools::file::GlobSearchTool,
    )?;
    plane.register(
        ToolPath::new(["content.search"])?,
        ToolRegistration::discoverable("content.search"),
        loong_tools::file::ContentSearchTool,
    )?;
    Ok(plane)
}
