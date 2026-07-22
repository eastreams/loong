use std::borrow::Cow;
use std::path::{Path, PathBuf};

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
    MemorySessionContext, MemoryWorkspaceContext,
};

const REQUIRED_CAPABILITIES: [Capability; 2] = [Capability::MemoryRead, Capability::MemoryWrite];

/// Flush derived memory state before a conversation window is compacted.
///
/// This is a mutating memory operation even when a selected memory system has
/// no compact hook: policy must authorize the attempt before the backend may
/// decide that the configured stage is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCompactAction {
    session_id: String,
    workspace_root: Option<PathBuf>,
}

impl MemoryCompactAction {
    pub(super) fn new(session_id: impl Into<String>, workspace_root: Option<&Path>) -> Self {
        Self {
            session_id: session_id.into(),
            workspace_root: workspace_root.map(Path::to_path_buf),
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }
}

impl ActionMeta for MemoryCompactAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.compact",
            operation: Cow::Borrowed("compact"),
            required_capabilities: Cow::Borrowed(&REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(format!("memory://session/{}", self.session_id).into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "session_id": self.session_id,
            "workspace_root": self.workspace_root,
        }))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryCompactAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryCompactAction> for MemoryCompactAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-compact-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &MemoryCompactAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.compact reached terminal allow policy".into()),
            reason: "memory compaction stage allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryWorkspaceContext + MemoryExecutionContext,
{
    pub async fn compact(
        self,
    ) -> Result<<C::Cx<'ctx> as MemoryExecutionContext>::CompactOutput, MemoryAccessError> {
        let action = MemoryCompactAction::new(
            self.ctx.memory_session_id(),
            self.ctx.memory_workspace_root(),
        );
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryCompactAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = Cx::CompactOutput;
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().compact(granted).await
    }
}
