use std::collections::BTreeSet;

use loong_runtime::{runtime::Runtime, tool_plane::ToolPath};
use serde::{Deserialize, Serialize};

use super::runtime_config;
use super::skills;
use super::tool_surface;
use super::{ToolView, runtime_tool_view_for_runtime_config};

#[derive(Debug, Clone)]
pub struct ToolRegistryEntry {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverableToolSurfaceSummary {
    #[serde(default)]
    pub visible_direct_tools: Vec<String>,
    pub hidden_tool_count: usize,
    #[serde(default)]
    pub hidden_tags: Vec<String>,
    #[serde(default)]
    pub hidden_surfaces: Vec<super::ToolSurfaceState>,
}

pub fn tool_registry(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
) -> Vec<ToolRegistryEntry> {
    tool_registry_with_config(runtime, Some(runtime_config::get_tool_runtime_config()))
}

pub fn tool_registry_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Vec<ToolRegistryEntry> {
    let default_runtime_config;
    let config = match config {
        Some(config) => config,
        None => {
            default_runtime_config = runtime_config::ToolRuntimeConfig::default();
            &default_runtime_config
        }
    };

    let runtime_view = runtime_tool_view_for_runtime_config(config);
    let visible_direct_states = tool_surface::visible_direct_tool_states_for_view(&runtime_view);
    let mut entries = Vec::new();

    for state in visible_direct_states {
        let summary = agent_visible_summary_for_direct_state(runtime, &state);
        let registry_entry = ToolRegistryEntry {
            name: state.surface_id,
            description: format!("{} {}", summary, state.usage_guidance),
        };
        entries.push(registry_entry);
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

pub fn capability_snapshot(runtime: Option<&Runtime<crate::context::AppContextFactory>>) -> String {
    capability_snapshot_with_config(runtime, runtime_config::get_tool_runtime_config())
}

pub fn capability_snapshot_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    config: &runtime_config::ToolRuntimeConfig,
) -> String {
    capability_snapshot_for_view_with_config(
        runtime,
        &runtime_tool_view_for_runtime_config(config),
        config,
    )
}

pub fn capability_snapshot_for_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    view: &ToolView,
) -> String {
    capability_snapshot_for_view_with_config(
        runtime,
        view,
        runtime_config::get_tool_runtime_config(),
    )
}

pub(crate) fn capability_snapshot_for_view_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    view: &ToolView,
    config: &runtime_config::ToolRuntimeConfig,
) -> String {
    let visible_direct_states = tool_surface::visible_direct_tool_states_for_view(view);
    capability_snapshot_for_direct_states_with_config(runtime, view, config, visible_direct_states)
}

pub(crate) fn capability_snapshot_for_direct_states_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    _view: &ToolView,
    config: &runtime_config::ToolRuntimeConfig,
    visible_direct_states: Vec<super::ToolSurfaceState>,
) -> String {
    let mut lines = vec![
        "[tool_discovery_runtime]".to_owned(),
        "Available tools:".to_owned(),
    ];

    let visible_direct_lines =
        render_visible_direct_tool_lines(runtime, visible_direct_states.as_slice());
    lines.extend(visible_direct_lines);

    lines.push("Guidelines:".to_owned());
    let hidden_surfaces = Vec::new();
    let guideline_lines = render_active_tool_guideline_lines(
        visible_direct_states.as_slice(),
        hidden_surfaces.as_slice(),
    );
    lines.extend(guideline_lines);
    if let Some(skill_catalog_section) = skills::model_skill_catalog_section_with_config(config) {
        lines.push(skill_catalog_section);
    }
    lines.join("\n")
}

fn render_visible_direct_tool_lines(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    states: &[super::ToolSurfaceState],
) -> Vec<String> {
    let mut lines = Vec::new();

    for state in states {
        let summary = agent_visible_summary_for_direct_state(runtime, state);
        let line = format!(
            "- {}: {} {}",
            state.surface_id, summary, state.usage_guidance
        );
        lines.push(line);
    }

    lines
}

fn agent_visible_summary_for_direct_state(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    state: &super::ToolSurfaceState,
) -> String {
    // Concrete typed tools own their action-level summary. The app surface keeps
    // usage guidance because it describes prompt/orchestration behavior, not the
    // tool's payload or side effect boundary.
    let typed_path = ToolPath::from(state.surface_id.as_str());
    runtime
        .and_then(|runtime| runtime.tools().spec(&typed_path).ok())
        .map(|spec| spec.description.clone())
        .unwrap_or_else(|| state.prompt_snippet.clone())
}

fn render_active_tool_guideline_lines(
    visible_direct_states: &[super::ToolSurfaceState],
    hidden_surfaces: &[super::ToolSurfaceState],
) -> Vec<String> {
    let mut lines = vec![
        "- Prefer a direct tool when one clearly fits.".to_owned(),
        "- If the user wants different permissions or guardrails, edit the relevant config or prompt files instead of treating the runtime as fixed.".to_owned(),
    ];
    let mut seen = lines.iter().cloned().collect::<BTreeSet<_>>();

    for surface in visible_direct_states.iter().chain(hidden_surfaces.iter()) {
        let Some(guidelines) =
            tool_surface::tool_surface_prompt_guidelines_for_id(surface.surface_id.as_str())
        else {
            continue;
        };
        for guideline in guidelines {
            let line = format!("- {guideline}");
            let inserted = seen.insert(line.clone());
            if inserted {
                lines.push(line);
            }
        }
    }

    lines
}

pub fn runtime_discoverable_tool_surface_summary_with_config(
    _config: &runtime_config::ToolRuntimeConfig,
    _visible_tool_view: Option<&ToolView>,
) -> DiscoverableToolSurfaceSummary {
    DiscoverableToolSurfaceSummary {
        visible_direct_tools: Vec::new(),
        hidden_tool_count: 0,
        hidden_tags: Vec::new(),
        hidden_surfaces: Vec::new(),
    }
}
