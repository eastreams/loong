use std::borrow::Cow;

use kernel::CapabilityToken;
use loong_contracts::{AuthorizationScope, AuthorizationSubject, Capabilities};
use loong_core::policy::context::{ContextFactory, PolicyContext};

#[derive(Debug, Clone, Copy)]
pub struct SpecContextFactory;

impl ContextFactory for SpecContextFactory {
    type Cx<'a> = SpecExecutionContext<'a>;
}

pub struct SpecExecutionContext<'a> {
    token: &'a CapabilityToken,
}

impl<'a> SpecExecutionContext<'a> {
    #[must_use]
    pub fn new(token: &'a CapabilityToken) -> Self {
        Self { token }
    }
}

impl PolicyContext for SpecExecutionContext<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        // Spec remains a legacy token context with no independently narrowed
        // authority, so the owned typed view stays at this boundary.
        Cow::Owned(self.token.allowed_capabilities.iter().copied().collect())
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: self.token.agent_id.clone(),
            scope: AuthorizationScope::LegacyToken {
                boundary: "spec".to_owned(),
                pack_id: self.token.pack_id.clone(),
                token_id: self.token.token_id.clone(),
            },
        }
    }
}
