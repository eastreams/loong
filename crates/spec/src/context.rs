use std::collections::BTreeSet;

use kernel::{Capability, CapabilityToken, KernelInvocationContext, VerticalPackManifest};
use loong_core::policy::context::{ContextFactory, PolicyContext};
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct SpecContextFactory;

impl ContextFactory for SpecContextFactory {
    type Cx<'a> = SpecExecutionContext<'a>;
}

pub struct SpecExecutionContext<'a> {
    pack: &'a VerticalPackManifest,
    token: &'a CapabilityToken,
    now_epoch_s: u64,
    request_parameters: Option<&'a Value>,
}

impl<'a> SpecExecutionContext<'a> {
    #[must_use]
    pub fn new(
        pack: &'a VerticalPackManifest,
        token: &'a CapabilityToken,
        now_epoch_s: u64,
        request_parameters: Option<&'a Value>,
    ) -> Self {
        Self {
            pack,
            token,
            now_epoch_s,
            request_parameters,
        }
    }
}

impl PolicyContext for SpecExecutionContext<'_> {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.token.allowed_capabilities.clone()
    }
}

impl KernelInvocationContext for SpecExecutionContext<'_> {
    fn pack(&self) -> &VerticalPackManifest {
        self.pack
    }

    fn token(&self) -> &CapabilityToken {
        self.token
    }

    fn now_epoch_s(&self) -> u64 {
        self.now_epoch_s
    }

    fn request_parameters(&self) -> Option<&Value> {
        self.request_parameters
    }
}
