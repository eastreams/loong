use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;

// Re-export data types from contracts
pub use loongclaw_contracts::{
    RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionOutcome, RuntimeExtensionRequest,
    RuntimeTier,
};

use crate::errors::RuntimePlaneError;

#[async_trait]
pub trait CoreRuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, RuntimePlaneError>;
}

#[async_trait]
pub trait RuntimeExtensionAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn execute_extension(
        &self,
        request: RuntimeExtensionRequest,
        core: &(dyn CoreRuntimeAdapter + Sync),
    ) -> Result<RuntimeExtensionOutcome, RuntimePlaneError>;
}

#[derive(Default)]
pub struct RuntimePlane {
    core_adapters: BTreeMap<String, Arc<dyn CoreRuntimeAdapter>>,
    extension_adapters: BTreeMap<String, Arc<dyn RuntimeExtensionAdapter>>,
    default_core_adapter: Option<String>,
}

impl RuntimePlane {
    #[must_use]
    pub fn new() -> Self {
        Self {
            core_adapters: BTreeMap::new(),
            extension_adapters: BTreeMap::new(),
            default_core_adapter: None,
        }
    }

    pub fn register_core_adapter<A: CoreRuntimeAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        if self.default_core_adapter.is_none() {
            self.default_core_adapter = Some(name.clone());
        }
        self.core_adapters.insert(name, Arc::new(adapter));
    }

    pub fn register_extension_adapter<A: RuntimeExtensionAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        self.extension_adapters.insert(name, Arc::new(adapter));
    }

    pub fn set_default_core_adapter(&mut self, name: &str) -> Result<(), RuntimePlaneError> {
        if !self.core_adapters.contains_key(name) {
            return Err(RuntimePlaneError::CoreAdapterNotFound(name.to_owned()));
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
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, RuntimePlaneError> {
        let resolved_name = if let Some(name) = core_name {
            name
        } else {
            self.default_core_adapter
                .as_deref()
                .ok_or(RuntimePlaneError::NoDefaultCoreAdapter)?
        };

        let adapter = self
            .core_adapters
            .get(resolved_name)
            .ok_or(RuntimePlaneError::CoreAdapterNotFound(
                resolved_name.to_owned(),
            ))?
            .clone();

        return adapter.execute_core(request).await;
    }

    pub async fn execute_extension(
        &self,
        extension_name: &str,
        core_name: Option<&str>,
        request: RuntimeExtensionRequest,
    ) -> Result<RuntimeExtensionOutcome, RuntimePlaneError> {
        let extension = self
            .extension_adapters
            .get(extension_name)
            .ok_or_else(|| RuntimePlaneError::ExtensionNotFound(extension_name.to_owned()))?
            .clone();

        let resolved_core_name = if let Some(name) = core_name {
            name
        } else {
            self.default_core_adapter
                .as_deref()
                .ok_or(RuntimePlaneError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(resolved_core_name)
            .ok_or(RuntimePlaneError::CoreAdapterNotFound(
                resolved_core_name.to_owned(),
            ))?
            .clone();

        return extension.execute_extension(request, core.as_ref()).await;
    }
}
