use std::collections::{BTreeMap, BTreeSet};

use loong_contracts::{ToolPath, ToolSpec};
use loong_runtime::{runtime::Runtime, tool_plane::error::LookupError};
use serde_json::{Value, json};

use super::error::ToolMetadataError;
use super::{
    ToolAvailability, ToolDescriptor, ToolView, catalog, runtime_config,
    runtime_tool_view_for_runtime_config, tool_catalog, tool_surface,
};

pub fn provider_tool_definitions(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
) -> Result<Vec<Value>, ToolMetadataError> {
    provider_tool_definitions_with_config(runtime, Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn provider_tool_definitions_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Result<Vec<Value>, ToolMetadataError> {
    let default_runtime_config;
    let config = match config {
        Some(config) => config,
        None => {
            default_runtime_config = runtime_config::ToolRuntimeConfig::default();
            &default_runtime_config
        }
    };

    let view = runtime_tool_view_for_runtime_config(config);
    provider_tool_definitions_for_view_with_config(runtime, &view)
}

pub fn try_provider_tool_definitions_for_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    view: &ToolView,
) -> Result<Vec<Value>, ToolMetadataError> {
    provider_tool_definitions_for_view_with_config(runtime, view)
}

fn provider_tool_definitions_for_view_with_config(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    view: &ToolView,
) -> Result<Vec<Value>, ToolMetadataError> {
    let catalog = tool_catalog();
    let typed_tool_paths = runtime
        .map(Runtime::registered_tool_paths)
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut tools = Vec::new();

    for descriptor in catalog.descriptors().iter() {
        if descriptor.availability != ToolAvailability::Runtime || !descriptor.is_provider_exposed()
        {
            continue;
        }

        let typed_path =
            ToolPath::new([descriptor.name]).map_err(|source| ToolMetadataError::InvalidPath {
                tool_name: descriptor.name.to_owned(),
                source,
            })?;
        if descriptor.requires_typed_plane_metadata() && !typed_tool_paths.contains(&typed_path) {
            continue;
        }

        if descriptor.is_direct()
            && !tool_surface::direct_tool_visible_in_view(descriptor.name, view)
        {
            continue;
        }

        tools.push(provider_definition_for_view(runtime, descriptor, view)?);
    }

    tools.sort_by(|left, right| tool_function_name(left).cmp(tool_function_name(right)));
    Ok(tools)
}

pub fn tool_parameter_schema_types() -> BTreeMap<String, BTreeMap<String, &'static str>> {
    let mut tools_by_name = BTreeMap::<String, BTreeMap<String, &'static str>>::new();
    for entry in catalog::all_tool_catalog() {
        let parameters = entry
            .parameter_types
            .iter()
            .map(|(parameter_name, parameter_type)| ((*parameter_name).to_owned(), *parameter_type))
            .collect::<BTreeMap<_, _>>();
        if !parameters.is_empty() {
            tools_by_name.insert(entry.canonical_name.to_owned(), parameters);
        }
    }
    tools_by_name
}

fn tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub(super) fn provider_definition_for_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> Result<Value, ToolMetadataError> {
    Ok(sanitize_provider_parameter_combinators(
        tool_metadata_definition_for_view(runtime, descriptor, view)?,
    ))
}

pub(super) fn tool_metadata_definition_for_view(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> Result<Value, ToolMetadataError> {
    // `tool.search` consumes this internal projection, so keep combinators that
    // describe payload variants. Provider submission sanitizes them above.
    let definition = typed_provider_definition_for_descriptor(runtime, descriptor)?
        .unwrap_or_else(|| descriptor.provider_definition());
    Ok(match descriptor.name {
        "web" => direct_web_provider_definition_for_view(definition, view),
        "browse" => direct_browser_provider_definition_for_view(definition, view),
        _ => definition,
    })
}

fn typed_provider_definition_for_descriptor(
    runtime: Option<&Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
) -> Result<Option<Value>, ToolMetadataError> {
    let Some(spec) = typed_tool_spec_for_descriptor(runtime, descriptor)? else {
        return Ok(None);
    };

    // Transitional boundary: app still wraps provider JSON, but migrated tools
    // own their input schema through ToolSpec instead of the legacy catalog.
    Ok(Some(json!({
        "type": "function",
        "function": {
            "name": descriptor.provider_name,
            "description": spec.description.clone(),
            "parameters": spec.input_schema.clone()
        }
    })))
}

pub(super) fn typed_tool_spec_for_descriptor<'a>(
    runtime: Option<&'a Runtime<crate::context::AppContextFactory>>,
    descriptor: &ToolDescriptor,
) -> Result<Option<&'a ToolSpec>, ToolMetadataError> {
    // Legacy descriptors still enumerate provider-visible tools, while
    // migrated metadata lives in the runtime plane under the same identity.
    let path =
        ToolPath::new([descriptor.name]).map_err(|source| ToolMetadataError::InvalidPath {
            tool_name: descriptor.name.to_owned(),
            source,
        })?;
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    match runtime.tool_spec(&path) {
        Ok(spec) => Ok(Some(spec)),
        Err(LookupError::NotRegistered { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn sanitize_provider_parameter_combinators(mut definition: Value) -> Value {
    let Some(function) = definition
        .get_mut("function")
        .and_then(Value::as_object_mut)
    else {
        return definition;
    };
    let Some(parameters) = function
        .get_mut("parameters")
        .and_then(Value::as_object_mut)
    else {
        return definition;
    };

    for key in ["allOf", "anyOf", "oneOf"] {
        parameters.remove(key);
    }

    definition
}

fn direct_web_provider_definition_for_view(mut definition: Value, view: &ToolView) -> Value {
    let web_runtime_modes = tool_surface::direct_web_runtime_modes_for_view(view);
    let ordinary_network_access_available = web_runtime_modes.ordinary_network_access_available();

    let Some(description) = web_runtime_modes.provider_description() else {
        return definition;
    };

    if let Some(function) = definition
        .get_mut("function")
        .and_then(Value::as_object_mut)
    {
        function.insert(
            "description".to_owned(),
            Value::String(description.to_owned()),
        );

        let Some(parameters) = function
            .get_mut("parameters")
            .and_then(Value::as_object_mut)
        else {
            return definition;
        };
        let Some(properties) = parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        else {
            return definition;
        };

        if !ordinary_network_access_available {
            for key in [
                "url",
                "mode",
                "max_bytes",
                "method",
                "headers",
                "body",
                "content_type",
            ] {
                properties.remove(key);
            }
        } else {
            if !web_runtime_modes.fetch_available {
                properties.remove("mode");
            }
            if !web_runtime_modes.request_available {
                for key in ["method", "headers", "body", "content_type"] {
                    properties.remove(key);
                }
            }
        }

        if !web_runtime_modes.query_search_available {
            for key in ["query", "provider", "max_results"] {
                properties.remove(key);
            }
        }

        parameters.remove("required");
        let mut any_of = Vec::new();
        if ordinary_network_access_available {
            any_of.push(json!({"required": ["url"]}));
        }
        if web_runtime_modes.query_search_available {
            any_of.push(json!({"required": ["query"]}));
        }

        match any_of.as_slice() {
            [] => {
                parameters.remove("anyOf");
            }
            [single] => {
                parameters.remove("anyOf");
                if let Some(required) = single.get("required") {
                    parameters.insert("required".to_owned(), required.clone());
                }
            }
            _ => {
                parameters.insert("anyOf".to_owned(), Value::Array(any_of));
            }
        }
    }

    definition
}

fn direct_browser_provider_definition_for_view(mut definition: Value, view: &ToolView) -> Value {
    let browser_runtime_modes = tool_surface::direct_browser_runtime_modes_for_view(view);
    let Some(description) = browser_runtime_modes.provider_description() else {
        return definition;
    };

    let function_value = definition.get_mut("function");
    let Some(function) = function_value.and_then(Value::as_object_mut) else {
        return definition;
    };

    function.insert(
        "description".to_owned(),
        Value::String(description.to_owned()),
    );

    let parameters_value = function.get_mut("parameters");
    let Some(parameters) = parameters_value.and_then(Value::as_object_mut) else {
        return definition;
    };

    let properties_value = parameters.get_mut("properties");
    let Some(properties) = properties_value.and_then(Value::as_object_mut) else {
        return definition;
    };

    let action_value = properties.get_mut("action");
    if let Some(action_property) = action_value.and_then(Value::as_object_mut) {
        action_property.insert("enum".to_owned(), json!(["open", "extract", "click"]));
        action_property.insert(
            "description".to_owned(),
            Value::String("Bounded page-browser action to perform.".to_owned()),
        );
    }

    let mode_value = properties.get_mut("mode");
    if let Some(mode_property) = mode_value.and_then(Value::as_object_mut) {
        mode_property.insert(
            "enum".to_owned(),
            json!(["page_text", "title", "links", "selector_text"]),
        );
        mode_property.insert(
            "description".to_owned(),
            Value::String("Extraction mode for the bounded browser page session.".to_owned()),
        );
    }

    definition
}
