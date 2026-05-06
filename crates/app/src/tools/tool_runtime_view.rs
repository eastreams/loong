use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{ToolView, runtime_config, runtime_tool_view_for_runtime_config, skills};

pub(crate) fn model_visible_skill_roots_for_runtime_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Vec<PathBuf> {
    skills::model_visible_skill_roots_with_config(config)
}

pub(crate) fn model_visible_skill_context_payload_for_path(
    config: &runtime_config::ToolRuntimeConfig,
    raw_path: &Path,
) -> Result<Option<Value>, String> {
    skills::model_visible_skill_context_payload_for_path(config, raw_path)
}

pub(crate) fn model_visible_skill_context_payload_for_skill_id(
    config: &runtime_config::ToolRuntimeConfig,
    skill_id: &str,
) -> Result<Option<Value>, String> {
    skills::model_visible_skill_context_payload_for_skill_id(config, skill_id)
}

pub fn runtime_tool_view_from_loong_config(config: &crate::config::LoongConfig) -> ToolView {
    let runtime_config = runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    runtime_tool_view_with_runtime_config(&config.tools, &runtime_config)
}

pub(crate) fn runtime_tool_view_with_runtime_config(
    _tool_config: &crate::config::ToolConfig,
    runtime_config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    runtime_tool_view_for_runtime_config(runtime_config)
}

/// Build a tool view from runtime config (respecting runtime toggles) plus
/// feishu entries when the feishu integration is configured. This avoids
/// using `ToolConfig::default()` which ignores runtime-disabled tools.
pub(crate) fn full_runtime_tool_view_for_runtime_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    runtime_tool_view_for_runtime_config(config)
}

pub(crate) fn effective_runtime_visible_tool_view(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
) -> ToolView {
    let runtime_view = full_runtime_tool_view_for_runtime_config(config);

    match visible_tool_view {
        Some(injected) => {
            // Intersect the injected view with the runtime-visible surface so that
            // trusted _loong.tool_search.visible_tool_ids cannot re-expose
            // tools disabled by runtime config (browser.*, session_*, etc.).
            injected.intersect(&runtime_view)
        }
        None => runtime_view,
    }
}
