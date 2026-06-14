pub use loong_contracts::{ToolOutcome, ToolRequest};

use async_trait::async_trait;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{errors::ToolPlaneError, tool::sealed::Sealed};

mod sealed {
    /// Prevent ToolAdapter from being implemented outside this crate
    pub trait Sealed {}
}

#[async_trait]
pub trait ToolAdapter: Send + Sync + Sealed {
    fn name(&self) -> &str;

    async fn execute(&self, request: ToolRequest) -> Result<ToolOutcome, ToolPlaneError>;
}

#[derive(Default)]
pub struct ToolPlane {
    adapters: BTreeMap<String, Arc<dyn ToolAdapter>>,
    default_adapter: Option<String>,
}

impl ToolPlane {
    #[must_use]
    pub fn new() -> Self {
        Self {
            adapters: BTreeMap::new(),
            default_adapter: None,
        }
    }

    pub fn register_core_adapter<A: ToolAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        if self.default_adapter.is_none() {
            self.default_adapter = Some(name.clone());
        }
        self.adapters.insert(name, Arc::new(adapter));
    }

    pub fn register_extension_adapter<A: ToolAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        self.adapters.insert(name, Arc::new(adapter));
    }

    pub fn set_default_core_adapter(&mut self, name: &str) -> Result<(), ToolPlaneError> {
        if !self.adapters.contains_key(name) {
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

        let adapter = self
            .adapters
            .get(resolved_name)
            .ok_or(ToolPlaneError::CoreAdapterNotFound(
                resolved_name.to_owned(),
            ))?
            .clone();

        return adapter.execute(request).await;
    }
}
