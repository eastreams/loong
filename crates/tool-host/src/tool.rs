use std::path::Path;

use async_trait::async_trait;
use contracts::tool::ToolSpec;
use kernel::{Facade, access::fs::FsAccess};
use schemars::{JsonSchema, Schema};
use serde_json::Value;
use thiserror::Error;

/// Per-call ambient parameters supplied by the agent.
#[derive(Debug, Clone)]
pub struct InvocationParams {
    pub workspace_root: std::path::PathBuf,
}

impl InvocationParams {
    #[must_use]
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }
}

/// Why a tool could not be registered.
#[derive(Debug, Error)]
pub enum RegistrationError {
    #[error("duplicate tool {0:?}")]
    Duplicate(String),
    #[error("invalid tool spec")]
    InvalidSpec,
}

/// Why a tool invocation failed.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool {0:?}")]
    UnknownTool(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(#[source] serde_json::Error),
    #[error("tool execution failed: {0}")]
    Execution(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("tool output serialization failed: {0}")]
    Output(#[source] serde_json::Error),
}

/// Root JSON Schema for a tool input or output type.
fn root_schema<T: JsonSchema>() -> Schema {
    let settings = schemars::generate::SchemaSettings::default().with(|settings| {
        settings.meta_schema = None;
        settings.inline_subschemas = true;
    });
    let generator = settings.into_generator();
    generator.into_root_schema_for::<T>()
}

/// The trusted handle supplied to one tool invocation.
pub trait ToolContext<H: ToolHost>: Sync {
    fn facade(&self) -> &Facade;
    fn workspace_root(&self) -> &Path;

    fn fs(&self) -> FsAccess<'_> {
        FsAccess::new(self.facade(), self.workspace_root())
    }
}

/// The tool host boundary the agent calls.
#[async_trait]
pub trait ToolHost: Send + Sync + Sized + 'static {
    type ToolCx<'a>: ToolContext<Self>
    where
        Self: 'a;

    async fn invoke(
        &self,
        name: &str,
        params: &InvocationParams,
        payload: Value,
    ) -> Result<Value, ToolError>;
}

/// A concrete tool implementation.
#[async_trait]
pub trait ToolImpl<H: ToolHost>: Send + Sync + 'static {
    type Input: JsonSchema + serde::de::DeserializeOwned + Send + 'static;
    type Output: JsonSchema + serde::Serialize + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn name(&self) -> &'static str;

    fn description(&self) -> &'static str;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().into(),
            description: self.description().into(),
            input_schema: root_schema::<Self::Input>(),
            output_schema: root_schema::<Self::Output>(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, serde_json::Error> {
        serde_json::from_value(payload)
    }

    async fn execute(
        &self,
        ctx: &H::ToolCx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error>;
}
