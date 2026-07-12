use std::path::PathBuf;

use crate::AppContext;
use crate::tools::ToolView;

use super::LoongConfig;

fn configured_root_session_workspace_root(config: &LoongConfig) -> Option<PathBuf> {
    config
        .tools
        .configured_runtime_workspace_root()
        .or_else(|| config.tools.configured_file_root())
        .and_then(|workspace_root| {
            let canonical_workspace_root = dunce::canonicalize(&workspace_root).ok()?;
            canonical_workspace_root
                .is_dir()
                .then_some(canonical_workspace_root)
        })
}

pub(super) fn root_session_context_from_config(
    app_ctx: &AppContext,
    config: &LoongConfig,
    session_id: impl Into<String>,
    tool_view: ToolView,
) -> AppContext {
    let mut session_context = app_ctx.for_session(session_id, tool_view);
    if let Some(workspace_root) = configured_root_session_workspace_root(config) {
        session_context = session_context.with_workspace_root(workspace_root);
    }
    session_context
}

pub(super) fn model_visible_skill_roots_from_config(config: &LoongConfig) -> Vec<PathBuf> {
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    crate::tools::model_visible_skill_roots_for_runtime_config(&tool_runtime_config)
}
