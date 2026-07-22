use crate::tools::catalog::{self, ToolDescriptor, ToolView};
use crate::tools::runtime_config;
use crate::tools::tool_surface;
use crate::tools::{
    SHELL_EXEC_TOOL_NAME, SearchableToolEntry, ToolAvailability, ToolMetadataError,
    runtime_tool_view_for_runtime_config, runtime_visible_tool_view,
};
use loong_contracts::ToolSpec;
use loong_runtime::{runtime::Runtime, tool_plane::ToolRegistration};

pub(crate) fn runtime_tool_search_entries(
    runtime: Option<&Runtime<crate::context::RuntimeContextFactory>>,
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    _collapse_hidden_surfaces: bool,
) -> Result<Vec<SearchableToolEntry>, ToolMetadataError> {
    let visible_tool_view = match runtime {
        Some(runtime) => runtime_visible_tool_view(runtime, config, visible_tool_view),
        None => {
            let configured = runtime_tool_view_for_runtime_config(config);
            match visible_tool_view {
                Some(injected) => injected.intersect(&configured),
                None => configured,
            }
        }
    };
    let mut entries = Vec::new();

    if let Some(runtime) = runtime {
        for path in runtime.registered_tool_paths() {
            let (registration, spec) = runtime.tool_metadata(&path)?;
            if !matches!(registration, ToolRegistration::Direct { .. }) {
                continue;
            }
            if !visible_tool_view.contains_path(&path) {
                continue;
            }
            entries.push(searchable_entry_from_registration(registration, spec));
        }
    }

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
            let entry = searchable_entry_from_descriptor_for_view(descriptor, &visible_tool_view);
            entries.push(entry);
        }
    }

    let hidden_entries =
        runtime_discoverable_tool_entries(runtime, config, Some(&visible_tool_view), true)?;
    entries.extend(hidden_entries);
    Ok(entries)
}

pub(crate) fn runtime_discoverable_tool_entries(
    runtime: Option<&Runtime<crate::context::RuntimeContextFactory>>,
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    provider_invokable_only: bool,
) -> Result<Vec<SearchableToolEntry>, ToolMetadataError> {
    let visible_tool_view = match runtime {
        Some(runtime) => runtime_visible_tool_view(runtime, config, visible_tool_view),
        None => {
            let configured = runtime_tool_view_for_runtime_config(config);
            match visible_tool_view {
                Some(injected) => injected.intersect(&configured),
                None => configured,
            }
        }
    };
    let mut entries = Vec::new();
    if let Some(runtime) = runtime {
        for path in runtime.registered_tool_paths() {
            let (registration, spec) = runtime.tool_metadata(&path)?;
            let ToolRegistration::Discoverable { discovery_name } = registration else {
                continue;
            };
            if !visible_tool_view.contains_path(&path)
                || !super::tool_search_entry_is_runtime_usable(discovery_name, config)
                || tool_surface::hidden_tool_is_covered_by_visible_direct_tool(
                    discovery_name,
                    &visible_tool_view,
                )
            {
                continue;
            }
            entries.push(searchable_entry_from_registration(registration, spec));
        }
    }
    for descriptor in catalog::tool_catalog().descriptors().iter() {
        if !descriptor.is_discoverable() {
            continue;
        }
        if provider_invokable_only
            && (!descriptor.is_provider_invokable_discoverable()
                || descriptor.name.starts_with("skills."))
        {
            continue;
        }
        if !visible_tool_view.contains(descriptor.name) {
            continue;
        }
        if descriptor.name != SHELL_EXEC_TOOL_NAME
            && !super::tool_search_entry_is_runtime_usable(descriptor.name, config)
        {
            continue;
        }
        if tool_surface::hidden_tool_is_covered_by_visible_direct_tool(
            descriptor.name,
            &visible_tool_view,
        ) {
            continue;
        }
        entries.push(searchable_entry_from_descriptor_for_view(
            descriptor,
            &visible_tool_view,
        ));
    }
    Ok(entries)
}

/// Project both direct and discoverable registered tools from one metadata owner.
fn searchable_entry_from_registration(
    registration: &ToolRegistration,
    spec: &ToolSpec,
) -> SearchableToolEntry {
    let search_hint = spec
        .search_hint
        .clone()
        .unwrap_or_else(|| spec.description.clone());
    let (canonical_name, provider_name, requires_lease) = match registration {
        ToolRegistration::Direct { provider_name } => {
            (provider_name.as_str(), provider_name.as_str(), false)
        }
        ToolRegistration::Discoverable { discovery_name } => {
            (discovery_name.as_str(), discovery_name.as_str(), true)
        }
    };
    super::searchable_entry_from_provider_definition(
        canonical_name,
        provider_name,
        &[],
        tool_surface::discovery_tool_name_for_tool_name(canonical_name),
        spec.description.clone(),
        search_hint,
        &spec.input_schema,
        &[],
        spec.argument_hint.clone(),
        spec.tags.clone(),
        tool_surface::tool_surface_id_for_name(canonical_name).map(str::to_owned),
        tool_surface::tool_surface_usage_guidance(canonical_name).map(str::to_owned),
        requires_lease,
    )
}

fn searchable_entry_from_descriptor_for_view(
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> SearchableToolEntry {
    let definition = crate::tools::legacy_tool_metadata_definition_for_view(descriptor, view);
    let function = definition.get("function");

    let summary_value = function.and_then(|value: &serde_json::Value| value.get("description"));
    let summary = summary_value
        .and_then(serde_json::Value::as_str)
        .unwrap_or(descriptor.description)
        .to_owned();

    let parameters_value = function.and_then(|value: &serde_json::Value| value.get("parameters"));
    let parameters = parameters_value.unwrap_or(&serde_json::Value::Null);
    let tags = descriptor
        .tags()
        .iter()
        .map(|tag| (*tag).to_owned())
        .collect::<Vec<_>>();
    let search_hint = direct_search_hint_for_runtime_view(descriptor, view)
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
        None,
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
