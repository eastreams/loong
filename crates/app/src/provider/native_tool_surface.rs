use std::collections::BTreeMap;

use loong_contracts::ToolPath;
use loong_runtime::runtime::Runtime;
use loong_runtime::tool_plane::ToolRegistration;
use serde_json::{Value, json};

use crate::config::{LoongConfig, ProviderKind, ProviderWireApi};
use crate::conversation::turn_engine::ToolIntentTarget;
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
    tool_definitions: Vec<Value>,
    targets: BTreeMap<String, ToolIntentTarget>,
}

impl ProviderToolRequestSurface {
    pub(super) fn new(
        tool_definitions: Vec<Value>,
        mut typed_targets: BTreeMap<String, ToolPath>,
    ) -> Result<Self, ProviderToolSurfaceError> {
        let mut targets = BTreeMap::new();
        for definition in &tool_definitions {
            let Some(provider_name) = definition
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let target = match typed_targets.remove(provider_name) {
                Some(path) => ToolIntentTarget::registered(path, provider_name),
                None => ToolIntentTarget::from(provider_name),
            };
            if targets.insert(provider_name.to_owned(), target).is_some() {
                return Err(ProviderToolSurfaceError::DuplicateProviderName {
                    provider_name: provider_name.to_owned(),
                });
            }
        }

        if let Some((provider_name, path)) = typed_targets.into_iter().next() {
            return Err(ProviderToolSurfaceError::MissingTypedDefinition {
                provider_name,
                path,
            });
        }
        Ok(Self {
            tool_definitions,
            targets,
        })
    }

    pub(super) fn definitions(&self) -> &[Value] {
        self.tool_definitions.as_slice()
    }

    pub(super) fn resolve(&self, provider_name: &str) -> Option<ToolIntentTarget> {
        self.targets.get(provider_name).cloned()
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ProviderToolSurfaceError {
    #[error(transparent)]
    Metadata(#[from] crate::tools::ToolMetadataError),
    #[error("provider tool name is emitted more than once: {provider_name}")]
    DuplicateProviderName { provider_name: String },
    #[error(
        "typed tool `{path}` has provider name `{provider_name}` but no emitted request definition"
    )]
    MissingTypedDefinition {
        provider_name: String,
        path: ToolPath,
    },
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
        runtime: &Runtime<crate::context::RuntimeContextFactory>,
        tool_view: &ToolView,
        tool_runtime_config: &tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<ProviderToolSurfacePlan, ProviderToolSurfaceError> {
        let runtime_tool_view = tools::runtime_tool_view_for_runtime_config(tool_runtime_config);
        let base_tool_definitions = if tool_view == &runtime_tool_view {
            tools::provider_tool_definitions_with_config(runtime, Some(tool_runtime_config))?
        } else {
            tools::provider_tool_definitions_for_view(runtime, tool_view)?
        };

        let request_tool_definitions = self
            .web_surface_mode
            .apply_to_tool_definitions(base_tool_definitions);
        let mut typed_targets = BTreeMap::new();
        for path in runtime.registered_tool_paths() {
            let (registration, _) = runtime
                .tool_metadata(&path)
                .map_err(crate::tools::ToolMetadataError::from)?;
            let ToolRegistration::Direct { provider_name } = registration else {
                continue;
            };
            if !tool_view.contains_path(&path) {
                continue;
            }
            if typed_targets.insert(provider_name.clone(), path).is_some() {
                return Err(ProviderToolSurfaceError::DuplicateProviderName {
                    provider_name: provider_name.clone(),
                });
            }
        }
        let request = ProviderToolRequestSurface::new(
            self.append_native_tool_specs(request_tool_definitions),
            typed_targets,
        )?;
        let direct_states = self.web_surface_mode.visible_direct_tool_states(tool_view);
        let capability_snapshot = tools::capability_snapshot_for_direct_states_with_config(
            Some(runtime),
            tool_view,
            tool_runtime_config,
            direct_states,
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_surface_preserves_typed_path_behind_distinct_provider_name() {
        let path = ToolPath::new(["typed.only"]).expect("test tool path must be valid");
        let surface = ProviderToolRequestSurface::new(
            vec![json!({
                "type": "function",
                "function": { "name": "typed_only", "parameters": { "type": "object" } }
            })],
            BTreeMap::from([("typed_only".to_owned(), path.clone())]),
        )
        .expect("request surface");

        assert_eq!(
            surface.resolve("typed_only"),
            Some(ToolIntentTarget::registered(path, "typed_only"))
        );
        assert_eq!(surface.resolve("typed.only"), None);
    }

    #[test]
    fn request_surface_rejects_duplicate_provider_names() {
        let definitions = vec![
            json!({"type": "function", "function": {"name": "duplicate"}}),
            json!({"type": "function", "function": {"name": "duplicate"}}),
        ];

        let error = ProviderToolRequestSurface::new(definitions, BTreeMap::new())
            .expect_err("wire names must be unique per request");

        assert!(matches!(
            error,
            ProviderToolSurfaceError::DuplicateProviderName { ref provider_name }
                if provider_name == "duplicate"
        ));
    }
}
