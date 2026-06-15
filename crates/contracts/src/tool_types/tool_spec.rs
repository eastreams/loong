use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Capability;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrigin {
    /// Built in Loong
    BuiltIn,
    /// Registered at compile time
    Extension,
    /// Registered at runtime
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSchedulingClass {
    /// The tool monopolize this program runtime. \
    /// The program should not do anything else running this tool.
    Exclusive,
    /// No two instances of this tool can run simultaneously.
    SerialOnly,
    /// This tool can run with any other task in parallel
    ParallelSafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolSpec {
    // --- Basic Info ---
    /// Should not include '.', '\' or '/'
    pub name: String,
    /// eg. loong, feishu, ...
    pub provider: String,
    /// A brief introduction to this tool
    pub description: String,
    /// Tool register location
    pub origin: ToolOrigin,

    // --- Runtime Info ---
    pub scheduling_class: ToolSchedulingClass,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<Capability>,
    // pub governance: ToolGovernanceProfile,

    // --- Calling Info ---
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    // path is managed by TRIE
    // visibility/governance is managed externally for mutability
}

impl ToolSpec {
    pub fn builder() -> ToolSpecBuilder {
        ToolSpecBuilder {
            name: None,
            provider: None,
            description: None,
            aliases: Vec::new(),
            origin: None,
            scheduling_class: None,
            required_capabilities: None,
            input_schema: None,
            output_schema: None,
        }
    }
}

pub struct ToolSpecBuilder {
    name: Option<String>,
    provider: Option<String>,
    description: Option<String>,
    aliases: Vec<String>,
    origin: Option<ToolOrigin>,
    scheduling_class: Option<ToolSchedulingClass>,
    required_capabilities: Option<Vec<Capability>>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
}

pub enum ToolExposureClass {
    Direct,
    Gateway,
    Discoverable,
}

impl ToolSpecBuilder {
    pub fn name(self, name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..self
        }
    }
    pub fn provider(self, provider: impl Into<String>) -> Self {
        Self {
            provider: Some(provider.into()),
            ..self
        }
    }
    pub fn description(self, description: impl Into<String>) -> Self {
        Self {
            description: Some(description.into()),
            ..self
        }
    }
    pub fn origin(self, tier: ToolOrigin) -> Self {
        Self {
            origin: Some(tier),
            ..self
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn scheduling_class(self, scheduling_class: ToolSchedulingClass) -> Self {
        Self {
            scheduling_class: Some(scheduling_class),
            ..self
        }
    }

    pub fn required_capabilities(self, caps: Vec<Capability>) -> Self {
        Self {
            required_capabilities: Some(caps),
            ..self
        }
    }

    pub fn input_schema(self, schema: Value) -> Self {
        Self {
            input_schema: Some(schema),
            ..self
        }
    }

    pub fn output_schema(self, schema: Value) -> Self {
        Self {
            output_schema: Some(schema),
            ..self
        }
    }

    pub fn build(self) -> Result<ToolSpec, ToolSpecBuildError> {
        let mut error = ToolSpecBuildError {
            missing_fields: Vec::new(),
            unsatisfied_constraints: Vec::new(),
        };

        match self.name {
            Some(name) => {
                if name.contains(&['.', '/', '\\']) {
                    error
                        .unsatisfied_constraints
                        .push("name should not contain '.', '/' or '\\'".to_string());
                }
            }
            None => {
                error.missing_fields.push("name".to_string());
            }
        }
        if self.provider.is_none() {
            error.missing_fields.push("provider".to_string());
        }
        if self.description.is_none() {
            error.missing_fields.push("description".to_string());
        }
        if self.origin.is_none() {
            error.missing_fields.push("tier".to_string());
        }
        if self.scheduling_class.is_none() {
            error.missing_fields.push("scheduling_class".to_string());
        }
        if self.required_capabilities.is_none() {
            error
                .missing_fields
                .push("required_capabilities".to_string());
        }
        match self.input_schema {
            Some(schema) => {
                if !schema.is_object() {
                    error
                        .unsatisfied_constraints
                        .push("input_schema should be an object".to_string());
                }
            }
            None => {
                error.missing_fields.push("input_schema".to_string());
            }
        }

        if !error.missing_fields.is_empty() || !error.unsatisfied_constraints.is_empty() {
            return Err(error);
        }

        Ok(ToolSpec {
            name: self.name.unwrap(),
            provider: self.provider.unwrap(),
            description: self.description.unwrap(),
            origin: self.origin.unwrap(),
            scheduling_class: self.scheduling_class.unwrap(),
            required_capabilities: self.required_capabilities.unwrap(),
            input_schema: self.input_schema.unwrap(),
            output_schema: self.output_schema,
        })
    }
}

#[derive(Debug)]
pub struct ToolSpecBuildError {
    pub unsatisfied_constraints: Vec<String>,
    pub missing_fields: Vec<String>,
}
