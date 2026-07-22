use std::{any::Any, borrow::Cow, collections::BTreeSet};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::policy::{action::ActionMeta, context::ContextFactory, policy::PolicyAny};
use loong_runtime::tool_plane::ToolInvocationAction;

use crate::config::{GovernedToolApprovalMode, ToolConfig, ToolConsentMode};

/// App-configured consent gate for typed tools that can mutate the filesystem.
///
/// The policy reads the typed invocation action before payload parsing or tool
/// execution. A permission decision is resolved by `PolicyEngine::grant`; this
/// policy neither persists legacy approval envelopes nor performs side effects.
#[derive(Debug, Clone)]
pub(crate) struct ToolMutationConsentPolicy {
    consent_mode: ToolConsentMode,
    approval_mode: GovernedToolApprovalMode,
    approved_calls: BTreeSet<String>,
    denied_calls: BTreeSet<String>,
}

impl From<&ToolConfig> for ToolMutationConsentPolicy {
    fn from(config: &ToolConfig) -> Self {
        Self {
            consent_mode: config.consent.default_mode,
            approval_mode: config.approval.mode,
            approved_calls: config.approval.approved_calls.iter().cloned().collect(),
            denied_calls: config.approval.denied_calls.iter().cloned().collect(),
        }
    }
}

#[async_trait]
impl<C> PolicyAny<C> for ToolMutationConsentPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tool-mutation-consent")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyGrant {
        let Some(action) = (action as &dyn Any).downcast_ref::<ToolInvocationAction>() else {
            return PolicyGrant {
                decision: PolicyDecision::Continue,
                predicate: Some("action is not a tool invocation".into()),
                reason: "tool mutation consent policy does not apply".into(),
            };
        };
        if !action
            .required_capabilities()
            .contains(&Capability::FilesystemWrite)
        {
            return PolicyGrant {
                decision: PolicyDecision::Continue,
                predicate: Some("tool invocation does not require filesystem mutation".into()),
                reason: "tool mutation consent is not required".into(),
            };
        }

        // Typed consent keys embed the canonical ToolPath. Legacy `tool:name`
        // keys remain owned by legacy ingress and cannot authorize this path.
        let approval_key = format!("tool:{}", action.path());
        if self.denied_calls.contains(&approval_key) {
            return PolicyGrant {
                decision: PolicyDecision::Deny,
                predicate: Some(format!("{approval_key} is denied by app policy").into()),
                reason: format!("typed mutation `{}` is denied by app policy", action.path())
                    .into(),
            };
        }
        if self.approved_calls.contains(&approval_key) {
            return PolicyGrant {
                decision: PolicyDecision::Continue,
                predicate: Some(format!("{approval_key} is preapproved by app policy").into()),
                reason: "typed mutation is preapproved".into(),
            };
        }

        let permission_reason = match self.consent_mode {
            ToolConsentMode::Prompt => Some("session consent mode requires confirmation"),
            ToolConsentMode::Auto => Some("filesystem mutation is not eligible for auto consent"),
            ToolConsentMode::Full => match self.approval_mode {
                GovernedToolApprovalMode::Disabled => None,
                GovernedToolApprovalMode::MediumBalanced | GovernedToolApprovalMode::Strict => {
                    Some("governed approval mode requires confirmation for filesystem mutation")
                }
            },
        };
        match permission_reason {
            Some(reason) => PolicyGrant {
                decision: PolicyDecision::RequireUserPermission,
                predicate: Some(reason.into()),
                reason: format!(
                    "user permission is required before invoking `{}`",
                    action.path()
                )
                .into(),
            },
            None => PolicyGrant {
                decision: PolicyDecision::Continue,
                predicate: Some("full consent with governed approvals disabled".into()),
                reason: "typed mutation consent allows policy evaluation to continue".into(),
            },
        }
    }
}
