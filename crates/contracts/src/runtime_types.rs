use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeTier {
    Core,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCoreRequest {
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCoreOutcome {
    pub status: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExtensionRequest {
    pub action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeExtensionOutcome {
    pub status: String,
    pub payload: Value,
}
