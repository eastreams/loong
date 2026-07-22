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
pub struct MemoryTranscriptAction {
    session_id: String,
    page_size: usize,
}

impl MemoryTranscriptAction {
    pub(super) fn new(session_id: impl Into<String>, page_size: usize) -> Self {
        Self {
            session_id: session_id.into(),
            page_size,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn page_size(&self) -> usize {
        self.page_size
    }
}

impl ActionMeta for MemoryTranscriptAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.transcript",
            operation: Cow::Borrowed("transcript"),
            required_capabilities: Cow::Borrowed(&REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(format!("memory://session/{}", self.session_id).into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "session_id": self.session_id,
            "page_size": self.page_size,
        }))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryTranscriptAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryTranscriptAction> for MemoryTranscriptAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-transcript-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &MemoryTranscriptAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.transcript reached terminal allow policy".into()),
            reason: "memory transcript read allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryExecutionContext,
{
    pub async fn transcript(self, page_size: usize) -> Result<MemorySnapshot, MemoryAccessError> {
        let action = MemoryTranscriptAction::new(self.ctx.memory_session_id(), page_size);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryTranscriptAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = MemorySnapshot;
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().transcript(granted).await
    }
}
