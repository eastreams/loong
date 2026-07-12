use loong_runtime::runtime::Runtime;
use serde_json::{Value, json};

use crate::config::{LoongConfig, ProviderKind, ProviderWireApi};
use crate::tools::{self, ToolSurfaceState, ToolView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderNativeToolKind {
    WebSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderNativePromptSection {
    pub(super) id: &'static str,
    pub(super) content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderToolRequestSurface {
    pub(super) tool_definitions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderToolPromptSurface {
    pub(super) capability_snapshot: String,
    pub(super) prompt_sections: Vec<ProviderNativePromptSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderToolSurfacePlan {
    pub(super) request: ProviderToolRequestSurface,
    pub(super) prompt: ProviderToolPromptSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderWebSurfaceMode {
    StandardQuerySearch,
    NativeQuerySearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderToolSurface {
    web_surface_mode: ProviderWebSurfaceMode,
    native_tools: &'static [ProviderNativeToolKind],
}

pub(super) fn provider_tool_surface(config: &LoongConfig) -> ProviderToolSurface {
    let native_query_search = config.tools.web_search.enabled
        && matches!(config.provider.kind, ProviderKind::Openai)
        && matches!(config.provider.wire_api, ProviderWireApi::Responses);
    if native_query_search {
        return ProviderToolSurface {
            web_surface_mode: ProviderWebSurfaceMode::NativeQuerySearch,
            native_tools: &[ProviderNativeToolKind::WebSearch],
        };
    }

    ProviderToolSurface {
        web_surface_mode: ProviderWebSurfaceMode::StandardQuerySearch,
        native_tools: &[],
    }
}

impl ProviderToolSurface {
    pub(super) fn native_query_search_active(self) -> bool {
        !self.native_tools.is_empty()
    }

    pub(super) fn native_query_search_label(self) -> Option<String> {
        if !self.native_query_search_active() {
            return None;
        }

        Some("OpenAI Responses native web search".to_owned())
    }

    pub(super) fn materialize(
        self,
        runtime: Option<&Runtime<crate::context::AppContextFactory>>,
        config: &LoongConfig,
        tool_view: &ToolView,
        tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<ProviderToolSurfacePlan, String> {
        let runtime_tool_view =
            tools::runtime_tool_view_with_runtime_config(&config.tools, tool_runtime_config);
        let base_tool_definitions = if tool_view == &runtime_tool_view {
            tools::provider_tool_definitions_with_config(runtime, Some(tool_runtime_config))
        } else {
            tools::try_provider_tool_definitions_for_view(runtime, tool_view)?
        };

        let request_tool_definitions = self
            .web_surface_mode
            .apply_to_tool_definitions(base_tool_definitions);
        let request = ProviderToolRequestSurface {
            tool_definitions: self.append_native_tool_specs(request_tool_definitions),
        };
        let direct_states = self.web_surface_mode.visible_direct_tool_states(tool_view);
        let capability_snapshot = tools::capability_snapshot_for_direct_states_with_config(
            runtime,
            tool_view,
            tool_runtime_config,
            direct_states,
        );
        let prompt_sections = self
            .native_tools
            .iter()
            .filter_map(|kind| kind.prompt_section())
            .collect();
        let prompt = ProviderToolPromptSurface {
            capability_snapshot,
            prompt_sections,
        };

        Ok(ProviderToolSurfacePlan { request, prompt })
    }

    fn append_native_tool_specs(self, mut tools: Vec<Value>) -> Vec<Value> {
        for kind in self.native_tools {
            tools.push(kind.request_tool_spec());
        }
        tools
    }
}

impl ProviderWebSurfaceMode {
    fn apply_to_tool_definitions(self, mut tools: Vec<Value>) -> Vec<Value> {
        if !matches!(self, Self::NativeQuerySearch) {
            return tools;
        }

        for tool in &mut tools {
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

        tools
    }

    fn visible_direct_tool_states(self, view: &ToolView) -> Vec<ToolSurfaceState> {
        let mut states = tools::visible_direct_tool_states_for_view(view);
        if !matches!(self, Self::NativeQuerySearch) {
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
}

impl ProviderNativeToolKind {
    fn request_tool_spec(self) -> Value {
        match self {
            Self::WebSearch => json!({ "type": "web_search" }),
        }
    }

    fn prompt_section(self) -> Option<ProviderNativePromptSection> {
        match self {
            Self::WebSearch => Some(ProviderNativePromptSection {
                id: "native-web-search",
                content: [
                    "## Native Query Search".to_owned(),
                    "- This OpenAI Responses profile exposes native `web_search` for query-style public web search."
                        .to_owned(),
                    "- Use native `web_search` for search queries."
                        .to_owned(),
                    "- Use `web` for direct URL fetches and low-level HTTP requests."
                        .to_owned(),
                ]
                .join("\n"),
            }),
        }
    }
}
