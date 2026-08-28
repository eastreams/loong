//! Tool contract: errors, the per-call [`ToolContext`], and the [`ToolImpl`]
//! trait shared by every tool implementation.

use std::path::Path;

use async_trait::async_trait;
use contracts::tool::ToolSpec;
use kernel::Facade;
use kernel::access::fs::FsAccess;
use kernel::resource::{Resource, Resources, WorkspaceRoot};
use schemars::{JsonSchema, Schema};
use serde_json::Value;
use thiserror::Error;

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
///
/// Tools read resources from the facade that the registry owns. The facade is
/// the same assembly boundary used for custom actions, while [`fs`](
/// ToolContext::fs) is the fixed workspace-scoped convenience for filesystem
/// tools.
pub struct ToolContext<'a> {
    facade: &'a Facade,
}

impl<'a> ToolContext<'a> {
    pub(crate) fn new(facade: &'a Facade) -> Self {
        Self { facade }
    }

    #[must_use]
    pub fn facade(&self) -> &Facade {
        self.facade
    }

    #[must_use]
    pub fn resources(&self) -> &Resources {
        self.facade.resources()
    }

    #[must_use]
    pub fn resource<R: Resource>(&self) -> Option<&R> {
        self.resources().get::<R>()
    }

    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        self.resource::<WorkspaceRoot>()
            .map(|root| root.0.as_path())
            .expect("WorkspaceRoot resource is missing from tool context")
    }

    #[must_use]
    pub fn fs(&self) -> FsAccess<'_> {
        FsAccess::new(self.facade, self.workspace_root())
    }
}

/// A concrete tool implementation.
#[async_trait]
pub trait ToolImpl: Send + Sync + 'static {
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
        ctx: &ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error>;
}
