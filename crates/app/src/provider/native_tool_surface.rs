use serde_json::{Value, json};

use crate::config::{LoongConfig, ProviderKind, ProviderWireApi};
use crate::tools::{self, ToolSurfaceState, ToolView};

pub(super) fn provider_request_tool_definitions(
    config: &LoongConfig,
    tool_view: &ToolView,
    tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<Value>, String> {
    let runtime_tool_view =
        tools::runtime_tool_view_with_runtime_config(&config.tools, tool_runtime_config);
    let base_tool_definitions = if tool_view == &runtime_tool_view {
        tools::provider_tool_definitions_with_config(Some(tool_runtime_config))
    } else {
        tools::try_provider_tool_definitions_for_view(tool_view)?
    };

    Ok(responses_tool_definitions_with_native_search(
        config,
        base_tool_definitions.as_slice(),
    ))
}

pub(super) fn openai_responses_native_web_search_active(config: &LoongConfig) -> bool {
    config.tools.web_search.enabled
        && matches!(config.provider.kind, ProviderKind::Openai)
        && matches!(config.provider.wire_api, ProviderWireApi::Responses)
}

pub(super) fn responses_tool_definitions_with_native_search(
    config: &LoongConfig,
    tool_definitions: &[Value],
) -> Vec<Value> {
    let mut tools = tool_definitions.to_vec();
    if !openai_responses_native_web_search_active(config) {
        return tools;
    }

    trim_function_web_query_mode_for_native_web_search(&mut tools);
    tools.push(json!({ "type": "web_search" }));
    tools
}

pub(super) fn native_web_search_prompt_section(config: &LoongConfig) -> Option<String> {
    if !openai_responses_native_web_search_active(config) {
        return None;
    }

    Some(
        [
            "## Native Query Search".to_owned(),
            "- This OpenAI Responses profile exposes native `web_search` for query-style public web search."
                .to_owned(),
            "- Use native `web_search` for search queries."
                .to_owned(),
            "- Use `web` for direct URL fetches and low-level HTTP requests."
                .to_owned(),
        ]
        .join("\n"),
    )
}

pub(super) fn provider_capability_snapshot(
    config: &LoongConfig,
    view: &ToolView,
    tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
) -> String {
    let direct_states = provider_visible_direct_tool_states(config, view);
    tools::capability_snapshot_for_direct_states_with_config(
        view,
        tool_runtime_config,
        direct_states,
    )
}

fn trim_function_web_query_mode_for_native_web_search(tools: &mut [Value]) {
    for tool in tools {
        let Some(function) = tool.get_mut("function").and_then(Value::as_object_mut) else {
            continue;
        };
        let tool_name = function.get("name").and_then(Value::as_str);
        if tool_name != Some("web") {
            continue;
        }

        function.insert(
            "description".to_owned(),
            Value::String("Fetch a URL or send HTTP requests".to_owned()),
        );

        let Some(parameters) = function
            .get_mut("parameters")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        let Some(properties) = parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };

        for key in ["query", "provider", "max_results"] {
            properties.remove(key);
        }

        parameters.remove("anyOf");
        parameters.insert("required".to_owned(), json!(["url"]));
    }
}

fn provider_visible_direct_tool_states(
    config: &LoongConfig,
    view: &ToolView,
) -> Vec<ToolSurfaceState> {
    let mut states = tools::visible_direct_tool_states_for_view(view);
    if !openai_responses_native_web_search_active(config) {
        return states;
    }

    for state in &mut states {
        if state.surface_id != "web" {
            continue;
        }

        state.prompt_snippet = "fetch a URL or send an HTTP request.".to_owned();
        state.usage_guidance =
            "Use web for direct URL fetches and low-level HTTP requests.".to_owned();
    }

    states
}
