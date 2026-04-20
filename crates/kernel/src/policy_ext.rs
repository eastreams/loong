use std::{collections::BTreeSet, sync::Arc};

use crate::{
    contracts::{Capability, CapabilityToken},
    errors::PolicyError,
    pack::VerticalPackManifest,
};

pub struct PolicyExtensionContext<'a> {
    pub pack: &'a VerticalPackManifest,
    pub token: &'a CapabilityToken,
    pub now_epoch_s: u64,
    pub required_capabilities: &'a BTreeSet<Capability>,
    pub request_parameters: Option<&'a serde_json::Value>,
}

pub trait PolicyExtension: Send + Sync {
    fn name(&self) -> &str;
    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError>;
}

#[derive(Default)]
pub struct PolicyExtensionChain {
    extensions: Vec<Arc<dyn PolicyExtension>>,
}

impl PolicyExtensionChain {
    #[must_use]
    pub fn new() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    pub fn register<E: PolicyExtension + 'static>(&mut self, extension: E) {
        self.extensions.push(Arc::new(extension));
    }

    pub fn authorize(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        for extension in &self.extensions {
            extension.authorize_extension(context)?;
        }
        Ok(())
    }
}
