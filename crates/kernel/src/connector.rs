use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;

use crate::{
    contracts::{ConnectorCommand, ConnectorOutcome},
    errors::ConnectorError,
};

#[async_trait]
pub trait ConnectorAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn invoke(&self, command: ConnectorCommand) -> Result<ConnectorOutcome, ConnectorError>;
}

#[derive(Default)]
pub struct ConnectorRegistry {
    connectors: BTreeMap<String, Arc<dyn ConnectorAdapter>>,
}

impl ConnectorRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            connectors: BTreeMap::new(),
        }
    }

    pub fn register<A: ConnectorAdapter + 'static>(&mut self, connector: A) {
        let key = connector.name().to_owned();
        self.connectors.insert(key, Arc::new(connector));
    }

    pub async fn invoke(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let connector_name = command.connector_name.clone();
        let connector = self
            .connectors
            .get(&connector_name)
            .ok_or(ConnectorError::NotFound(connector_name))?
            .clone();
        connector.invoke(command).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorTier {
    Core,
    Extension,
}

#[async_trait]
pub trait CoreConnectorAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError>;
}

#[async_trait]
pub trait ConnectorExtensionAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn invoke_extension(
        &self,
        command: ConnectorCommand,
        core: &(dyn CoreConnectorAdapter + Sync),
    ) -> Result<ConnectorOutcome, ConnectorError>;
}

#[derive(Default)]
pub struct ConnectorPlane {
    core_adapters: BTreeMap<String, Arc<dyn CoreConnectorAdapter>>,
    extension_adapters: BTreeMap<String, Arc<dyn ConnectorExtensionAdapter>>,
    default_core_adapter: Option<String>,
}

impl ConnectorPlane {
    #[must_use]
    pub fn new() -> Self {
        Self {
            core_adapters: BTreeMap::new(),
            extension_adapters: BTreeMap::new(),
            default_core_adapter: None,
        }
    }

    pub fn register_core_adapter<A: CoreConnectorAdapter + 'static>(&mut self, adapter: A) {
        let name = adapter.name().to_owned();
        self.core_adapters.insert(name.clone(), Arc::new(adapter));
        if self.default_core_adapter.is_none() {
            self.default_core_adapter = Some(name);
        }
    }

    pub fn register_extension_adapter<A: ConnectorExtensionAdapter + 'static>(
        &mut self,
        adapter: A,
    ) {
        let name = adapter.name().to_owned();
        self.extension_adapters.insert(name, Arc::new(adapter));
    }

    pub fn set_default_core_adapter(&mut self, name: &str) -> Result<(), ConnectorError> {
        if !self.core_adapters.contains_key(name) {
            return Err(ConnectorError::CoreAdapterNotFound(name.to_owned()));
        }
        self.default_core_adapter = Some(name.to_owned());
        Ok(())
    }

    #[must_use]
    pub fn default_core_adapter_name(&self) -> Option<&str> {
        self.default_core_adapter.as_deref()
    }

    pub async fn invoke_core(
        &self,
        core_name: Option<&str>,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let resolved_name = if let Some(name) = core_name {
            name.to_owned()
        } else {
            self.default_core_adapter
                .clone()
                .ok_or(ConnectorError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(&resolved_name)
            .ok_or(ConnectorError::CoreAdapterNotFound(resolved_name))?
            .clone();

        core.invoke_core(command).await
    }

    pub async fn invoke_extension(
        &self,
        extension_name: &str,
        core_name: Option<&str>,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let extension = self
            .extension_adapters
            .get(extension_name)
            .ok_or_else(|| ConnectorError::ExtensionNotFound(extension_name.to_owned()))?
            .clone();

        let resolved_core_name = if let Some(name) = core_name {
            name.to_owned()
        } else {
            self.default_core_adapter
                .clone()
                .ok_or(ConnectorError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(&resolved_core_name)
            .ok_or(ConnectorError::CoreAdapterNotFound(resolved_core_name))?
            .clone();

        extension.invoke_extension(command, core.as_ref()).await
    }
}
