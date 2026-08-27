//! SSE chunk parsing and streaming tool-call assembly.

use serde::Deserialize;

/// Accumulates one tool call as its SSE deltas arrive.
#[derive(Default)]
pub(super) struct ToolCallBuilder {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) arguments: String,
}

/// Applies one chunk's tool-call deltas to the in-flight builders.
pub(super) fn apply_tool_deltas(
    builders: &mut Vec<ToolCallBuilder>,
    deltas: &[OpenAiToolCallDelta],
) {
    for delta in deltas {
        if builders.len() <= delta.index {
            builders.resize_with(delta.index + 1, ToolCallBuilder::default);
        }
        let builder = &mut builders[delta.index];
        if let Some(id) = delta.id.as_deref()
            && builder.id.is_none()
        {
            builder.id = Some(id.to_owned());
        }
        if let Some(function) = delta.function.as_ref() {
            if let Some(name) = function.name.as_deref()
                && builder.name.is_none()
            {
                builder.name = Some(name.to_owned());
            }
            if let Some(arguments) = function.arguments.as_deref() {
                builder.arguments.push_str(arguments);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiChunk {
    pub(super) choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiChoice {
    pub(super) delta: Option<OpenAiDelta>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiDelta {
    pub(super) content: Option<String>,
    pub(super) tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiToolCallDelta {
    pub(super) index: usize,
    pub(super) id: Option<String>,
    pub(super) function: Option<OpenAiFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenAiFunctionDelta {
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}
