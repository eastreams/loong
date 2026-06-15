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

impl ToolOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Extension => "extension",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffectClass {
    ReadOnly,
    Mutating,
}

impl ToolEffectClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutating => "mutating",
        }
    }
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

impl ToolSchedulingClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::SerialOnly => "serial_only",
            Self::ParallelSafe => "parallel_safe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposureClass {
    Direct,
    Gateway,
    Discoverable,
}

impl ToolExposureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Gateway => "gateway",
            Self::Discoverable => "discoverable",
        }
    }
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
    pub effect_class: ToolEffectClass,
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
            effect_class: None,
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
    effect_class: Option<ToolEffectClass>,
    scheduling_class: Option<ToolSchedulingClass>,
    required_capabilities: Option<Vec<Capability>>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
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

    pub fn effect_class(self, effect_class: ToolEffectClass) -> Self {
        Self {
            effect_class: Some(effect_class),
            ..self
        }
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
        let ToolSpecBuilder {
            name,
            provider,
            description,
            aliases: _,
            origin,
            effect_class,
            scheduling_class,
            required_capabilities,
            input_schema,
            output_schema,
        } = self;

        let mut error = ToolSpecBuildError {
            missing_fields: Vec::new(),
            unsatisfied_constraints: Vec::new(),
        };

        match &name {
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
        if provider.is_none() {
            error.missing_fields.push("provider".to_string());
        }
        if description.is_none() {
            error.missing_fields.push("description".to_string());
        }
        if origin.is_none() {
            error.missing_fields.push("tier".to_string());
        }
        if effect_class.is_none() {
            error.missing_fields.push("effect_class".to_string());
        }
        if scheduling_class.is_none() {
            error.missing_fields.push("scheduling_class".to_string());
        }
        if matches!(
            (effect_class, scheduling_class),
            (
                Some(ToolEffectClass::Mutating),
                Some(ToolSchedulingClass::ParallelSafe)
            )
        ) {
            error
                .unsatisfied_constraints
                .push("mutating tools should not use parallel_safe scheduling".to_string());
        }
        if required_capabilities.is_none() {
            error
                .missing_fields
                .push("required_capabilities".to_string());
        }
        match &input_schema {
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

        match (
            name,
            provider,
            description,
            origin,
            effect_class,
            scheduling_class,
            required_capabilities,
            input_schema,
        ) {
            (
                Some(name),
                Some(provider),
                Some(description),
                Some(origin),
                Some(effect_class),
                Some(scheduling_class),
                Some(required_capabilities),
                Some(input_schema),
            ) if error.missing_fields.is_empty() && error.unsatisfied_constraints.is_empty() => {
                Ok(ToolSpec {
                    name,
                    provider,
                    description,
                    origin,
                    effect_class,
                    scheduling_class,
                    required_capabilities,
                    input_schema,
                    output_schema,
                })
            }
            _ => Err(error),
        }
    }
}

#[derive(Debug)]
pub struct ToolSpecBuildError {
    pub unsatisfied_constraints: Vec<String>,
    pub missing_fields: Vec<String>,
}
