use serde_json::{Value, json};

use crate::config::{LoongConfig, ProviderKind, ProviderWireApi};

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
