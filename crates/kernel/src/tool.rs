use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;

// Re-export data types from contracts
pub use loongclaw_contracts::{
    ToolCoreOutcome, ToolCoreRequest, ToolExtensionOutcome, ToolExtensionRequest, ToolTier,
};

use crate::errors::ToolPlaneError;

#[async_trait]
pub trait CoreToolAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError>;
}

#[async_trait]
pub trait ToolExtensionAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, ToolPlaneError>;
}

#[derive(Default)]
pub struct ToolPlane {
    core_adapters: BTreeMap<String, Arc<dyn CoreToolAdapter>>,
    extension_adapters: BTreeMap<String, Arc<dyn ToolExtensionAdapter>>,
    default_core_adapter: Option<String>,
}

impl ToolPlane {
    #[must_use]
    pub fn new() -> Self {
        Self {
            core_adapters: BTreeMap::new(),
            extension_adapters: BTreeMap::new(),
            default_core_adapter: None,
        }
    }

    pub fn register_core_adapter<A: CoreToolAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        self.core_adapters.insert(name.clone(), Arc::new(adapter));
        if self.default_core_adapter.is_none() {
            self.default_core_adapter = Some(name);
        }
    }

    pub fn register_extension_adapter<A: ToolExtensionAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        self.extension_adapters.insert(name, Arc::new(adapter));
    }

    pub fn set_default_core_adapter(&mut self, name: &str) -> Result<(), ToolPlaneError> {
        if !self.core_adapters.contains_key(name) {
            return Err(ToolPlaneError::CoreAdapterNotFound(name.to_owned()));
        }
        self.default_core_adapter = Some(name.to_owned());
        Ok(())
    }

    #[must_use]
    pub fn default_core_adapter_name(&self) -> Option<&str> {
        self.default_core_adapter.as_deref()
    }

    pub async fn execute_core(
        &self,
        core_name: Option<&str>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, ToolPlaneError> {
        let resolved_name = if let Some(name) = core_name {
            name.to_owned()
        } else {
            self.default_core_adapter
                .clone()
                .ok_or(ToolPlaneError::NoDefaultCoreAdapter)?
        };

        let adapter = self
            .core_adapters
            .get(&resolved_name)
            .ok_or(ToolPlaneError::CoreAdapterNotFound(resolved_name))?
            .clone();

        adapter.execute_core_tool(request).await
    }

    pub async fn execute_extension(
        &self,
        extension_name: &str,
        core_name: Option<&str>,
        request: ToolExtensionRequest,
    ) -> Result<ToolExtensionOutcome, ToolPlaneError> {
        let extension = self
            .extension_adapters
            .get(extension_name)
            .ok_or_else(|| ToolPlaneError::ExtensionNotFound(extension_name.to_owned()))?
            .clone();

        let resolved_core_name = if let Some(name) = core_name {
            name.to_owned()
        } else {
            self.default_core_adapter
                .clone()
                .ok_or(ToolPlaneError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(&resolved_core_name)
            .ok_or(ToolPlaneError::CoreAdapterNotFound(resolved_core_name))?
            .clone();

        extension
            .execute_tool_extension(request, core.as_ref())
            .await
    }
}
