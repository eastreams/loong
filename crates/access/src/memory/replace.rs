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
    MemorySessionContext, MemoryTurn,
};

const REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::MemoryWrite];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReplaceTurnsAction {
    session_id: String,
    turns: Vec<MemoryTurn>,
    expected_turn_count: Option<usize>,
}

impl MemoryReplaceTurnsAction {
    pub(super) fn new(
        session_id: impl Into<String>,
        turns: Vec<MemoryTurn>,
        expected_turn_count: Option<usize>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turns,
            expected_turn_count,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn turns(&self) -> &[MemoryTurn] {
        &self.turns
    }

    #[must_use]
    pub fn expected_turn_count(&self) -> Option<usize> {
        self.expected_turn_count
    }
}

impl ActionMeta for MemoryReplaceTurnsAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.replace_turns",
            operation: Cow::Borrowed("replace_turns"),
            required_capabilities: Cow::Borrowed(&REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(format!("memory://session/{}", self.session_id).into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "session_id": self.session_id,
            "turns": self.turns.iter().map(|turn| json!({
                "role": turn.role,
                "content": turn.content,
                "ts": turn.ts,
            })).collect::<Vec<_>>(),
            "expected_turn_count": self.expected_turn_count,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryReplaceTurnsOutcome {
    Replaced,
    Conflict,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryReplaceTurnsAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryReplaceTurnsAction> for MemoryReplaceTurnsAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-replace-turns-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &MemoryReplaceTurnsAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.replace_turns reached terminal allow policy".into()),
            reason: "memory window replacement allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryExecutionContext,
{
    pub async fn replace_turns(
        self,
        turns: Vec<MemoryTurn>,
        expected_turn_count: Option<usize>,
    ) -> Result<MemoryReplaceTurnsOutcome, MemoryAccessError> {
        let action =
            MemoryReplaceTurnsAction::new(self.ctx.memory_session_id(), turns, expected_turn_count);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryReplaceTurnsAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = MemoryReplaceTurnsOutcome;
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().replace_turns(granted).await
    }
}
