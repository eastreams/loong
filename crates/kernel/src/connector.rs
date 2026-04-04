use std::{any::Any, collections::BTreeMap, panic::AssertUnwindSafe, sync::Arc};

use async_trait::async_trait;
use futures_util::FutureExt;

use crate::{
    contracts::{ConnectorCommand, ConnectorOutcome},
    errors::ConnectorError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorTier {
    Core,
    Extension,
}

impl ConnectorTier {
    const fn as_error_scope(self) -> &'static str {
        match self {
            Self::Core => "connector core adapter",
            Self::Extension => "connector extension adapter",
        }
    }
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

struct PanicIsolatedCoreConnector {
    adapter_name: String,
    adapter: Arc<dyn CoreConnectorAdapter>,
}

impl PanicIsolatedCoreConnector {
    fn new(adapter_name: String, adapter: Arc<dyn CoreConnectorAdapter>) -> Self {
        Self {
            adapter_name,
            adapter,
        }
    }
}

#[async_trait]
impl CoreConnectorAdapter for PanicIsolatedCoreConnector {
    fn name(&self) -> &str {
        &self.adapter_name
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let invocation = self.adapter.invoke_core(command);
        return execute_connector_invocation(&self.adapter_name, ConnectorTier::Core, invocation)
            .await;
    }
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
        if self.default_core_adapter.is_none() {
            self.default_core_adapter = Some(name.clone());
        }
        self.core_adapters.insert(name, Arc::new(adapter));
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
            name
        } else {
            self.default_core_adapter
                .as_deref()
                .ok_or(ConnectorError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(resolved_name)
            .ok_or_else(|| ConnectorError::CoreAdapterNotFound(resolved_name.to_owned()))?
            .clone();

        let invocation = core.invoke_core(command);
        return execute_connector_invocation(resolved_name, ConnectorTier::Core, invocation).await;
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
            name
        } else {
            self.default_core_adapter
                .as_deref()
                .ok_or(ConnectorError::NoDefaultCoreAdapter)?
        };

        let core = self
            .core_adapters
            .get(resolved_core_name)
            .ok_or_else(|| ConnectorError::CoreAdapterNotFound(resolved_core_name.to_owned()))?
            .clone();

        let guarded_core = PanicIsolatedCoreConnector::new(resolved_core_name.to_owned(), core);
        let invocation = extension.invoke_extension(command, &guarded_core);
        return execute_connector_invocation(extension_name, ConnectorTier::Extension, invocation)
            .await;
    }
}

async fn execute_connector_invocation<F>(
    adapter_name: &str,
    tier: ConnectorTier,
    invocation: F,
) -> Result<ConnectorOutcome, ConnectorError>
where
    F: std::future::Future<Output = Result<ConnectorOutcome, ConnectorError>>,
{
    let guarded_invocation = AssertUnwindSafe(invocation);
    let panic_result = guarded_invocation.catch_unwind().await;

    match panic_result {
        Ok(outcome) => outcome,
        Err(panic_payload) => {
            let panic_message =
                format_connector_invocation_panic(adapter_name, tier, panic_payload);
            Err(ConnectorError::Execution(panic_message))
        }
    }
}

fn format_connector_invocation_panic(
    adapter_name: &str,
    tier: ConnectorTier,
    panic_payload: Box<dyn Any + Send>,
) -> String {
    let panic_message = extract_connector_panic_message(panic_payload);
    let scope = tier.as_error_scope();

    match panic_message {
        Some(message) => format!("{scope} `{adapter_name}` panicked: {message}"),
        None => format!("{scope} `{adapter_name}` panicked"),
    }
}

fn extract_connector_panic_message(panic_payload: Box<dyn Any + Send>) -> Option<String> {
    let panic_payload = match panic_payload.downcast::<String>() {
        Ok(message) => return Some(*message),
        Err(panic_payload) => panic_payload,
    };

    match panic_payload.downcast::<&'static str>() {
        Ok(message) => Some((*message).to_owned()),
        Err(_) => None,
    }
}
