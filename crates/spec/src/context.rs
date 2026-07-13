use std::borrow::Cow;

use kernel::{CapabilityToken, KernelInvocationContext, VerticalPackManifest};
use loong_contracts::Capabilities;
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
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        // Spec remains a legacy token context with no independently narrowed
        // authority, so the owned typed view stays at this boundary.
        Cow::Owned(self.token.allowed_capabilities.iter().copied().collect())
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
