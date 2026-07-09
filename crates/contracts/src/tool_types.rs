use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::Capability;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolPath(String);

impl ToolPath {
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for ToolPath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for ToolPath {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub path: ToolPath,
    pub description: String,
    pub required_capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: Value,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ToolInputError {
    #[error("missing tool input field `{field}`")]
    MissingField { field: String },
    #[error("invalid tool input field `{field}`: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("invalid tool input: {reason}")]
    InvalidPayload { reason: String },
}

impl ToolInputError {
    #[must_use]
    pub fn missing_field(field: impl Into<String>) -> Self {
        Self::MissingField {
            field: field.into(),
        }
    }

    #[must_use]
    pub fn invalid_field(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidField {
            field: field.into(),
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn invalid_payload(reason: impl Into<String>) -> Self {
        Self::InvalidPayload {
            reason: reason.into(),
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ToolExecutionError {
    #[error(transparent)]
    Input(#[from] ToolInputError),
    #[error("tool execution failed: {reason}")]
    Execution { reason: String },
}

impl ToolExecutionError {
    #[must_use]
    pub fn execution(reason: impl Into<String>) -> Self {
        Self::Execution {
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolTier {
    Core,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCoreRequest {
    pub tool_name: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCoreOutcome {
    pub status: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExtensionRequest {
    pub extension_action: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExtensionOutcome {
    pub status: String,
    pub payload: Value,
}
