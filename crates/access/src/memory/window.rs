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
    MemorySessionContext, MemorySnapshot,
};

const REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::MemoryRead];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWindowAction {
    session_id: String,
    limit: usize,
    allow_extended_limit: bool,
}

impl MemoryWindowAction {
    pub(super) fn new(
        session_id: impl Into<String>,
        limit: usize,
        allow_extended_limit: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            limit,
            allow_extended_limit,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    #[must_use]
    pub fn allow_extended_limit(&self) -> bool {
        self.allow_extended_limit
    }
}

impl ActionMeta for MemoryWindowAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.window",
            operation: Cow::Borrowed("window"),
            required_capabilities: Cow::Borrowed(&REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(format!("memory://session/{}", self.session_id).into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "session_id": self.session_id,
            "limit": self.limit,
            "allow_extended_limit": self.allow_extended_limit,
        }))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryWindowAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryWindowAction> for MemoryWindowAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-window-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &MemoryWindowAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.window reached terminal allow policy".into()),
            reason: "memory window read allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryExecutionContext,
{
    pub async fn window(
        self,
        limit: usize,
        allow_extended_limit: bool,
    ) -> Result<MemorySnapshot, MemoryAccessError> {
        let action =
            MemoryWindowAction::new(self.ctx.memory_session_id(), limit, allow_extended_limit);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryWindowAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = MemorySnapshot;
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().window(granted).await
    }
}
