mod tool_spec;

pub use tool_spec::{ToolOrigin, ToolSpec};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The Absolute path of a tool
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPath {
    pub segments: Vec<String>,
}

impl ToolPath {
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

#[deprecated]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCoreRequest {
    pub tool_name: String,
    pub payload: Value,
}

#[deprecated]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCoreOutcome {
    pub status: String,
    pub payload: Value,
}

#[deprecated]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExtensionRequest {
    pub extension_action: String,
    pub payload: Value,
}

#[deprecated]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExtensionOutcome {
    pub status: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRequest {
    #[serde(default, skip_serializing_if = "ToolPath::is_root")]
    pub tool_path: ToolPath,
    pub tool_name: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: Value,
}
