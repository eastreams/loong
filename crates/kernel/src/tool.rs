use loong_contracts::ToolSpec;
pub use loong_contracts::{ToolOutcome, ToolRequest};
mod tool_impl;

use async_trait::async_trait;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{errors::ToolPlaneError, tool::sealed::Sealed};

mod sealed {
    /// Prevent ToolAdapter from being implemented outside this crate
    pub trait Sealed {}
}

// TODO
type ToolContext = ();

#[async_trait]
pub trait ToolAdapter: Send + Sync + Sealed {
    async fn execute(
        &self,
        ctx: &ToolContext,
        request: ToolRequest,
    ) -> Result<ToolOutcome, ToolPlaneError>;
}

pub struct Tool {
    spec: ToolSpec,
    adapter: Arc<dyn ToolAdapter>,
}

#[derive(Default)]
pub struct ToolPlane {
    tools: BTreeMap<String, Tool>,
    default_adapter: Option<String>,
}

impl ToolPlane {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            default_adapter: None,
        }
    }

    // TODO: Add path
    pub fn register_core_tool(&mut self, spec: ToolSpec, adapter: Arc<dyn ToolAdapter>) {
        if self.default_adapter.is_none() {
            self.default_adapter = Some(spec.name.clone());
        }
        self.tools.insert(spec.name.clone(), Tool { spec, adapter });
    }

    // TODO: Add path
    pub fn register_extension_adapter(&mut self, spec: ToolSpec, adapter: Arc<dyn ToolAdapter>) {
        self.tools.insert(spec.name.clone(), Tool { spec, adapter });
    }

    pub fn set_default_core_adapter(&mut self, name: &str) -> Result<(), ToolPlaneError> {
        if !self.tools.contains_key(name) {
            return Err(ToolPlaneError::CoreAdapterNotFound(name.to_owned()));
        }
        self.default_adapter = Some(name.to_owned());
        Ok(())
    }

    #[must_use]
    pub fn default_adapter_name(&self) -> Option<&str> {
        self.default_adapter.as_deref()
    }

    pub async fn execute(
        &self,
        core_name: Option<&str>,
        request: ToolRequest,
    ) -> Result<ToolOutcome, ToolPlaneError> {
        let resolved_name = if let Some(name) = core_name {
            name
        } else {
            self.default_adapter
                .as_deref()
                .ok_or(ToolPlaneError::NoDefaultCoreAdapter)?
        };

        let tool = self
            .tools
            .get(resolved_name)
            .ok_or(ToolPlaneError::CoreAdapterNotFound(
                resolved_name.to_owned(),
            ))?;

        return tool.adapter.execute(&(), request).await;
    }
}
