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

const REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::MemoryRead];

/// Read the configured prompt-assembly memory projection for this Session.
///
/// Access captures the Session workspace root into the Action before policy
/// evaluation. There is deliberately no caller-supplied root argument: the
/// granted value is the backend's immutable execution input.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryReadStageEnvelopeAction {
    session_id: String,
    workspace_root: Option<PathBuf>,
}

impl MemoryReadStageEnvelopeAction {
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

impl ActionMeta for MemoryReadStageEnvelopeAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "memory.read_stage_envelope",
            operation: Cow::Borrowed("read_stage_envelope"),
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
pub struct MemoryReadStageEnvelopeAllowPolicy;

#[async_trait]
impl<C> Policy<C, MemoryReadStageEnvelopeAction> for MemoryReadStageEnvelopeAllowPolicy
where
    C: ContextFactory,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("memory-read-stage-envelope-allow")
    }

    async fn grant(
        &self,
        _ctx: &C::Cx<'_>,
        _action: &MemoryReadStageEnvelopeAction,
    ) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("memory.read_stage_envelope reached terminal allow policy".into()),
            reason: "memory stage envelope read allowed".into(),
        }
    }
}

impl<'a, 'ctx, C, P> MemoryAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C> + ?Sized,
    C::Cx<'ctx>: MemorySessionContext + MemoryWorkspaceContext + MemoryExecutionContext,
{
    pub async fn read_stage_envelope(
        self,
    ) -> Result<<C::Cx<'ctx> as MemoryExecutionContext>::StageEnvelope, MemoryAccessError> {
        let action = MemoryReadStageEnvelopeAction::new(
            self.ctx.memory_session_id(),
            self.ctx.memory_workspace_root(),
        );
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await.map_err(Into::into)
    }
}

#[async_trait]
impl<Cx> Action<Cx> for MemoryReadStageEnvelopeAction
where
    Cx: MemoryExecutionContext + ?Sized,
{
    type Output = Cx::StageEnvelope;
    type Error = MemoryBackendError;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error> {
        ctx.memory_backend().read_stage_envelope(granted).await
    }
}
