use loong_runtime::{runtime::Runtime, tool_plane::ToolRegistration};
use serde_json::{Value, json};

use super::{
    ToolAvailability, ToolDescriptor, ToolMetadataError, ToolView, runtime_config,
    runtime_visible_tool_view, tool_catalog, tool_surface,
};

pub fn provider_tool_definitions(
    runtime: &Runtime<crate::context::RuntimeContextFactory>,
) -> Result<Vec<Value>, ToolMetadataError> {
    provider_tool_definitions_with_config(runtime, Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn provider_tool_definitions_with_config(
    runtime: &Runtime<crate::context::RuntimeContextFactory>,
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

    let view = runtime_visible_tool_view(runtime, config, None);
    provider_tool_definitions_for_view(runtime, &view)
}

pub fn provider_tool_definitions_for_view(
    runtime: &Runtime<crate::context::RuntimeContextFactory>,
    view: &ToolView,
) -> Result<Vec<Value>, ToolMetadataError> {
    let catalog = tool_catalog();
    let mut tools = Vec::new();

    for path in runtime.registered_tool_paths() {
        let (registration, spec) = runtime.tool_metadata(&path)?;
        let ToolRegistration::Direct { provider_name } = registration else {
            continue;
        };
        if !view.contains_path(&path) {
            continue;
        }
        tools.push(sanitize_provider_parameter_combinators(json!({
            "type": "function",
            "function": {
                "name": provider_name,
                "description": spec.description,
                "parameters": spec.input_schema
            }
        })));
    }

    for descriptor in catalog.descriptors().iter() {
        if descriptor.availability != ToolAvailability::Runtime || !descriptor.is_provider_exposed()
        {
            continue;
        }

        if descriptor.is_direct()
            && !tool_surface::direct_tool_visible_in_view(descriptor.name, view)
        {
            continue;
        }

        tools.push(sanitize_provider_parameter_combinators(
            legacy_tool_metadata_definition_for_view(descriptor, view),
        ));
    }

    tools.sort_by(|left, right| tool_function_name(left).cmp(tool_function_name(right)));
    Ok(tools)
}

fn tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub(super) fn legacy_tool_metadata_definition_for_view(
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> Value {
    // Search keeps schema combinators; provider submission sanitizes the same
    // legacy projection at its external boundary.
    let definition = descriptor.provider_definition();
    match descriptor.name {
        "web" => direct_web_provider_definition_for_view(definition, view),
        "browse" => direct_browser_provider_definition_for_view(definition, view),
        _ => definition,
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
