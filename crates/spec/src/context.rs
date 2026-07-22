use std::borrow::Cow;

use loong_contracts::{AuthorizationScope, AuthorizationSubject, Capabilities};
use loong_core::policy::context::{ContextFactory, PolicyContext};

#[derive(Debug, Clone, Copy)]
pub struct SpecContextFactory;

impl ContextFactory for SpecContextFactory {
    type Cx<'a> = SpecExecutionContext<'a>;
}

/// Policy projection for one spec operation.
///
/// Legacy spec ingress may still execute with a bearer token, but that token
/// remains with the operation owner. Policy sees only these non-forgeable
/// authority facts and cannot recover or forward bearer evidence.
pub struct SpecExecutionContext<'a> {
    allowed_capabilities: Cow<'a, Capabilities>,
    authorization_subject: Cow<'a, AuthorizationSubject>,
}

impl SpecExecutionContext<'static> {
    #[must_use]
    pub fn from_legacy_token(token: &kernel::CapabilityToken) -> Self {
        Self {
            allowed_capabilities: Cow::Owned(token.allowed_capabilities.iter().copied().collect()),
            authorization_subject: Cow::Owned(AuthorizationSubject {
                actor_id: token.agent_id.clone(),
                scope: AuthorizationScope::LegacyToken {
                    boundary: "spec".to_owned(),
                    pack_id: token.pack_id.clone(),
                    token_id: token.token_id.clone(),
                },
            }),
        }
    }
}

impl PolicyContext for SpecExecutionContext<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(self.allowed_capabilities.as_ref())
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        self.authorization_subject.as_ref().clone()
    }
}
