use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::{CapabilityToken, ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use crate::config::{LoongConfig, ToolConfig};
use crate::context::RuntimeContextFactory;
use crate::session::store::SessionStoreConfig;
use loong_runtime::runtime::Runtime;

use super::super::autonomy_policy::AutonomyTurnBudgetState;
use super::support::{approval_required_tool_decision, generic_allow_tool_decision};
use super::{ApprovalRequirement, Context, LegacyToolPreflightOutcome, ToolIntent};

/// Owner of a request that reached the explicit legacy fallback.
///
/// Typed registrations return before this choice is made. Keeping typed
/// execution out of this enum prevents legacy preflight and replay protocols
/// from becoming part of the registered-tool contract again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyToolDispatchKind {
    LegacyCore,
    LegacyApp,
}

#[async_trait]
pub(crate) trait LegacyToolDispatcher: Send + Sync {
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        None
    }

    async fn preflight_tool_intent(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        execution_request: &ToolCoreRequest,
        trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        dispatch_kind: LegacyToolDispatchKind,
        _budget_state: &AutonomyTurnBudgetState,
    ) -> Result<LegacyToolPreflightOutcome, String> {
        match self
            .maybe_require_approval(
                session_context,
                intent,
                execution_request,
                trusted_internal_context,
                descriptor,
                dispatch_kind,
            )
            .await
        {
            Ok(Some(requirement)) => {
                let decision = approval_required_tool_decision(descriptor.name, &requirement);
                Ok(LegacyToolPreflightOutcome::NeedsApproval {
                    requirement,
                    decision,
                })
            }
            Ok(None) => {
                let decision = generic_allow_tool_decision(descriptor.name);
                Ok(LegacyToolPreflightOutcome::Allow(decision))
            }
            Err(reason) => Err(reason),
        }
    }

    async fn maybe_require_approval(
        &self,
        session_context: &Context<'_>,
        intent: &ToolIntent,
        _execution_request: &ToolCoreRequest,
        _trusted_internal_context: bool,
        descriptor: &crate::tools::ToolDescriptor,
        _dispatch_kind: LegacyToolDispatchKind,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let _ = (session_context, intent, descriptor);
        Ok(None)
    }

    async fn preflight_tool_execution(
        &self,
        _session_context: &Context<'_>,
        _intent: &ToolIntent,
        request: ToolCoreRequest,
        _descriptor: &crate::tools::ToolDescriptor,
    ) -> Result<LegacyToolExecutionPreflight, String> {
        Ok(LegacyToolExecutionPreflight::ready(request))
    }

    async fn execute_core_tool(
        &self,
        _session_context: &Context<'_>,
        request: ToolCoreRequest,
        _trusted_internal_context: bool,
    ) -> Result<ToolCoreOutcome, crate::tools::LegacyToolRequestError> {
        // A legacy fallback owner may implement only the app or core family.
        // Missing ownership is an explicit leaf error, never a typed fallback.
        Err(crate::tools::LegacyToolRequestError::Input(format!(
            "core_tool_not_implemented: {}",
            request.tool_name
        )))
    }

    /// Legacy-only app dispatch boundary.
    ///
    /// Bearer evidence is owned by the concrete fallback implementation and
    /// never enters preflight, TurnEngine, or registered typed invocation.
    async fn execute_app_tool(
        &self,
        session_context: &Context<'_>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, String>;

    async fn after_tool_execution(
        &self,
        _session_context: &Context<'_>,
        _intent: &ToolIntent,
        _intent_sequence: usize,
        _request: &ToolCoreRequest,
        _outcome: &ToolCoreOutcome,
    ) {
    }
}

#[cfg(test)]
pub(crate) struct NoopLegacyToolDispatcher;

#[cfg(test)]
#[async_trait]
impl LegacyToolDispatcher for NoopLegacyToolDispatcher {
    async fn execute_app_tool(
        &self,
        _session_context: &Context<'_>,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!("app_tool_not_implemented: {}", request.tool_name))
    }
}

pub(crate) enum LegacyToolExecutionPreflight {
    Ready {
        request: ToolCoreRequest,
        trusted_internal_context: bool,
    },
    NeedsApproval(ApprovalRequirement),
}

impl LegacyToolExecutionPreflight {
    pub(crate) fn ready(request: ToolCoreRequest) -> Self {
        Self::Ready {
            request,
            trusted_internal_context: false,
        }
    }
}

#[derive(Clone)]
pub struct DefaultLegacyToolDispatcher {
    /// Long-lived typed runtime owner used only when a legacy app tool detaches
    /// work beyond the borrowed Context lifetime.
    pub(super) execution_runtime: Arc<Runtime<RuntimeContextFactory>>,
    pub(super) memory_config: SessionStoreConfig,
    pub(super) tool_config: ToolConfig,
    pub(super) app_config: Option<Arc<LoongConfig>>,
    pub(super) legacy_token: CapabilityToken,
}

impl DefaultLegacyToolDispatcher {
    /// Clone the long-lived runtime owner only for detached legacy work.
    pub(crate) fn execution_runtime(&self) -> &Arc<Runtime<RuntimeContextFactory>> {
        &self.execution_runtime
    }

    /// Derive the legacy fallback owner for a concrete child Session.
    ///
    /// The executor configuration and Runtime remain shared, while bearer
    /// evidence is reissued from the child identity and capability ceiling.
    pub(crate) fn for_session(&self, session: &crate::Session) -> Result<Self, String> {
        let legacy_token = crate::legacy_kernel::issue_session_token(
            self.execution_runtime.as_ref(),
            session,
            crate::legacy_kernel::DEFAULT_LEGACY_TOKEN_TTL_S,
        )?;
        Ok(Self {
            execution_runtime: Arc::clone(&self.execution_runtime),
            memory_config: self.memory_config.clone(),
            tool_config: self.tool_config.clone(),
            app_config: self.app_config.clone(),
            legacy_token,
        })
    }

    /// Construct the explicit legacy fallback for one already-owned Session.
    ///
    /// Bearer evidence is minted and retained inside this owner; callers on the
    /// typed path never receive or forward it.
    pub(crate) fn new(
        execution_runtime: Arc<Runtime<RuntimeContextFactory>>,
        session: &crate::Session,
        memory_config: SessionStoreConfig,
        tool_config: ToolConfig,
    ) -> Result<Self, String> {
        let legacy_token = crate::legacy_kernel::issue_session_token(
            execution_runtime.as_ref(),
            session,
            crate::legacy_kernel::DEFAULT_LEGACY_TOKEN_TTL_S,
        )?;
        Ok(Self {
            execution_runtime,
            memory_config,
            tool_config,
            app_config: None,
            legacy_token,
        })
    }

    pub fn with_config(
        execution_runtime: Arc<Runtime<RuntimeContextFactory>>,
        session: &crate::Session,
        memory_config: SessionStoreConfig,
        app_config: LoongConfig,
    ) -> Result<Self, String> {
        let legacy_token = crate::legacy_kernel::issue_session_token(
            execution_runtime.as_ref(),
            session,
            crate::legacy_kernel::DEFAULT_LEGACY_TOKEN_TTL_S,
        )?;
        Ok(Self {
            execution_runtime,
            memory_config,
            tool_config: app_config.tools.clone(),
            app_config: Some(Arc::new(app_config)),
            legacy_token,
        })
    }

    /// Inject a deliberately controlled token into legacy-plane tests.
    #[cfg(test)]
    pub(crate) fn from_test_token(
        execution_runtime: Arc<Runtime<RuntimeContextFactory>>,
        memory_config: SessionStoreConfig,
        tool_config: ToolConfig,
        legacy_token: CapabilityToken,
    ) -> Self {
        Self {
            execution_runtime,
            memory_config,
            tool_config,
            app_config: None,
            legacy_token,
        }
    }
}

pub(super) enum LegacyGovernedToolPreflight {
    Allowed,
    AllowedWithTrustedInternalContext(Value),
    NeedsApproval(ApprovalRequirement),
}
