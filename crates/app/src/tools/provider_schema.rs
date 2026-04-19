use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    ToolAvailability, ToolDescriptor, ToolView, catalog, runtime_config,
    runtime_tool_view_for_runtime_config, tool_catalog, tool_surface,
};

pub fn provider_tool_definitions() -> Vec<Value> {
    provider_tool_definitions_with_config(Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn provider_tool_definitions_with_config(
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Vec<Value> {
    let default_runtime_config;
    let config = match config {
        Some(config) => config,
        None => {
            default_runtime_config = runtime_config::ToolRuntimeConfig::default();
            &default_runtime_config
        }
    };

    let view = runtime_tool_view_for_runtime_config(config);
    provider_tool_definitions_for_view_with_config(&view)
}

pub fn try_provider_tool_definitions_for_view(view: &ToolView) -> Result<Vec<Value>, String> {
    Ok(provider_tool_definitions_for_view_with_config(view))
}

fn provider_tool_definitions_for_view_with_config(view: &ToolView) -> Vec<Value> {
    let catalog = tool_catalog();
    let mut tools = Vec::new();

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

        tools.push(provider_definition_for_view(descriptor, view));
    }

    tools.sort_by(|left, right| tool_function_name(left).cmp(tool_function_name(right)));
    tools
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

pub(super) fn provider_definition_for_view(descriptor: &ToolDescriptor, view: &ToolView) -> Value {
    let definition = descriptor.provider_definition();
    let definition = if descriptor.name == "web" {
        direct_web_provider_definition_for_view(definition, view)
    } else {
        definition
    };

    sanitize_provider_parameter_combinators(definition)
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
