use crate::tools::catalog::{self, ToolDescriptor, ToolView};
use crate::tools::runtime_config;
use crate::tools::tool_surface;
use crate::tools::{
    SHELL_EXEC_TOOL_NAME, SearchableToolEntry, ToolAvailability,
    effective_runtime_visible_tool_view,
};
use loong_runtime::runtime::Runtime;

#[cfg(test)]
pub(crate) fn tool_id_visible_in_view(tool_id: &str, view: &ToolView) -> bool {
    let canonical_tool_id = super::canonical_tool_name(tool_id);
    if view.contains(canonical_tool_id) {
        return true;
    }

    if tool_surface::is_tool_surface_id(tool_id) {
        return tool_surface::tool_surface_visible_in_view(tool_id, view);
    }

    tool_surface::tool_surface_id_for_name(canonical_tool_id)
        .is_some_and(|surface_id| tool_surface::tool_surface_visible_in_view(surface_id, view))
}

pub(crate) fn runtime_tool_search_entries(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    _collapse_hidden_surfaces: bool,
) -> Vec<SearchableToolEntry> {
    let visible_tool_view = effective_runtime_visible_tool_view(config, visible_tool_view);
    let mut entries = Vec::new();

    for descriptor in catalog::tool_catalog().descriptors().iter() {
        let runtime_available = descriptor.availability == ToolAvailability::Runtime;
        if !runtime_available {
            continue;
        }

        if descriptor.is_direct() {
            let direct_tool_visible =
                tool_surface::direct_tool_visible_in_view(descriptor.name, &visible_tool_view);
            if !direct_tool_visible {
                continue;
            }
            let entry = searchable_entry_from_descriptor_for_runtime_view(
                runtime,
                descriptor,
                &visible_tool_view,
            );
            entries.push(entry);
        }
    }

    let hidden_entries =
        runtime_discoverable_tool_entries(runtime, config, Some(&visible_tool_view), true);
    entries.extend(hidden_entries);
    entries
}

pub(crate) fn runtime_discoverable_tool_entries(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    provider_invokable_only: bool,
) -> Vec<SearchableToolEntry> {
    let visible_tool_view = effective_runtime_visible_tool_view(config, visible_tool_view);
    catalog::tool_catalog()
        .descriptors()
        .iter()
        .filter(|descriptor| {
            let is_discoverable = descriptor.is_discoverable();
            if !is_discoverable {
                return false;
            }

            if !provider_invokable_only {
                return true;
            }

            descriptor.is_provider_invokable_discoverable()
                && !descriptor.name.starts_with("skills.")
        })
        .filter(|descriptor| visible_tool_view.contains(descriptor.name))
        .filter(|descriptor| {
            descriptor.name == SHELL_EXEC_TOOL_NAME
                || super::tool_search_entry_is_runtime_usable(descriptor.name, config)
        })
        .filter(|descriptor| {
            !tool_surface::hidden_tool_is_covered_by_visible_direct_tool(
                descriptor.name,
                &visible_tool_view,
            )
        })
        .map(|descriptor| {
            searchable_entry_from_descriptor_for_runtime_view(
                runtime,
                descriptor,
                &visible_tool_view,
            )
        })
        .collect::<Vec<_>>()
}

fn searchable_entry_from_descriptor_for_runtime_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> SearchableToolEntry {
    searchable_entry_from_descriptor_for_view(runtime, descriptor, view)
}

fn searchable_entry_from_descriptor_for_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> SearchableToolEntry {
    let definition = crate::tools::tool_metadata_definition_for_view(runtime, descriptor, view);
    let function = definition.get("function");

    let summary_value = function.and_then(|value: &serde_json::Value| value.get("description"));
    let summary = summary_value
        .and_then(serde_json::Value::as_str)
        .unwrap_or(descriptor.description)
        .to_owned();

    let parameters_value = function.and_then(|value: &serde_json::Value| value.get("parameters"));
    let parameters = parameters_value.unwrap_or(&serde_json::Value::Null);
    let typed_spec = crate::tools::typed_tool_spec_for_descriptor(runtime, descriptor);
    let tags = typed_spec
        .map(|spec| spec.tags.clone())
        .filter(|tags| !tags.is_empty())
        .unwrap_or_else(|| {
            descriptor
                .tags()
                .iter()
                .map(|tag| (*tag).to_owned())
                .collect::<Vec<_>>()
        });
    let search_hint = direct_search_hint_for_runtime_view(descriptor, view)
        .or_else(|| typed_spec.and_then(|spec| spec.search_hint.clone()))
        .unwrap_or_else(|| descriptor.search_hint().to_owned());
    let surface_id = descriptor.surface_id().map(str::to_owned);
    let usage_guidance = direct_usage_guidance_for_runtime_view(descriptor, view)
        .or_else(|| descriptor.usage_guidance().map(str::to_owned));
    let requires_lease = !descriptor.is_provider_exposed();
    let tool_id = tool_surface::discovery_tool_name_for_tool_name(descriptor.name);

    super::searchable_entry_from_provider_definition(
        descriptor.name,
        descriptor.provider_name,
        descriptor.aliases,
        tool_id,
        summary,
        search_hint,
        parameters,
        descriptor.parameter_types(),
        typed_spec.and_then(|spec| spec.argument_hint.clone()),
        tags,
        surface_id,
        usage_guidance,
        requires_lease,
    )
}

fn direct_search_hint_for_runtime_view(
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> Option<String> {
    match descriptor.name {
        "web" => {
            let web_runtime_modes = tool_surface::direct_web_runtime_modes_for_view(view);
            let search_hint = web_runtime_modes.search_hint()?;
            Some(search_hint.to_owned())
        }
        _ => None,
    }
}

fn direct_usage_guidance_for_runtime_view(
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> Option<String> {
    if !descriptor.is_direct() {
        return None;
    }

    tool_surface::visible_direct_tool_states_for_view(view)
        .into_iter()
        .find(|state| state.surface_id == descriptor.name)
        .map(|state| state.usage_guidance)
}
