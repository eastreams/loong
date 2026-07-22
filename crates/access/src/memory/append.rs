use std::borrow::Cow;

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::policy::{
    action::{Action, ActionMeta, ActionMetadata},
    context::ContextFactory,
    engine::PolicyEngine,
    grant::Granted,
    policy::Policy,
};
use serde_json::{Value, json};

use super::{
    MemoryAccess, MemoryAccessError, MemoryBackend, MemoryBackendError, MemoryExecutionContext,
    MemorySessionContext,
};

const REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::MemoryWrite];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAppendTurnAction {
    session_id: String,
    role: String,
    content: String,
}

impl MemoryAppendTurnAction {
    pub(super) fn new(session_id: impl Into<String>, role: &str, content: &str) -> Self {
        Self {
            session_id: session_id.into(),
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

impl ActionMeta for MemoryAppendTurnAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.append_turn",
            operation: Cow::Borrowed("append_turn"),
            required_capabilities: Cow::Borrowed(&REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(format!("memory://session/{}", self.session_id).into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "session_id": self.session_id,
            "role": self.role,
            "content": self.content,
        }))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryAppendTurnAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryAppendTurnAction> for MemoryAppendTurnAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-append-turn-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &MemoryAppendTurnAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.append_turn reached terminal allow policy".into()),
            reason: "memory turn append allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryExecutionContext,
{
    pub async fn append_turn(self, role: &str, content: &str) -> Result<(), MemoryAccessError> {
        let action = MemoryAppendTurnAction::new(self.ctx.memory_session_id(), role, content);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryAppendTurnAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = ();
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().append_turn(granted).await
    }
}
