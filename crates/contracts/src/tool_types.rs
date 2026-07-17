use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::Capability;

/// How orchestration may schedule independent invocations of a tool.
///
/// This metadata describes execution ordering only. It does not grant any
/// capability and must not be used as an authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSchedulingClass {
    ParallelSafe,
    SerialOnly,
}

impl ToolSchedulingClass {
    /// Stable label used by execution telemetry and operator-facing metadata.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParallelSafe => "parallel_safe",
            Self::SerialOnly => "serial_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub description: String,
    /// JSON Schema for the payload accepted by this tool.
    ///
    /// The registry path/function name is intentionally not part of the schema;
    /// app-owned planes provide identity, while concrete tools describe input.
    pub input_schema: Value,
    pub required_capabilities: BTreeSet<Capability>,
    /// Execution ordering metadata for orchestration, never authorization input.
    pub scheduling: ToolSchedulingClass,
    /// Optional compact argument hint owned by the concrete tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// Optional discovery text owned by the concrete tool, not the registry path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_hint: Option<String>,
    /// Optional search tags owned by the concrete tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolTier {
    Core,
    Extension,
}

// TODO(deprecate-tool-core-envelope): add #[deprecated] once app call sites use
// ctx.tool(path)?.invoke(payload).await directly. These legacy bridge envelopes
// are not the typed ToolImpl API; typed tools return Result<Value, E>.
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
