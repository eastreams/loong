use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use crate::config::{LoongConfig, ToolConfig};
use crate::session::store::{self, SessionStoreConfig};

use super::super::autonomy_policy::AutonomyTurnBudgetState;
use super::super::runtime_binding::ConversationRuntimeBinding;
use super::support::{approval_required_tool_decision, generic_allow_tool_decision};
use super::{ApprovalRequirement, SessionContext, ToolIntent, ToolPreflightOutcome};

#[async_trait]
pub trait AppToolDispatcher: Send + Sync {
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        None
    }

    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        _budget_state: &AutonomyTurnBudgetState,
    ) -> Result<ToolPreflightOutcome, String> {
        match self
            .maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
        {
            Ok(Some(requirement)) => {
                let decision = approval_required_tool_decision(descriptor.name, &requirement);
                Ok(ToolPreflightOutcome::NeedsApproval {
                    requirement,
                    decision,
                })
            }
            Ok(None) => {
                let decision = generic_allow_tool_decision(descriptor.name);
                Ok(ToolPreflightOutcome::Allow(decision))
            }
            Err(reason) => Err(reason),
        }
    }

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let _ = (session_context, intent, descriptor, binding);
        Ok(None)
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        _session_context: &SessionContext,
        _intent: &ToolIntent,
        request: ToolCoreRequest,
        _descriptor: &crate::tools::ToolDescriptor,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        Ok(ToolExecutionPreflight::ready(request))
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String>;

    async fn after_tool_execution(
        &self,
        _session_context: &SessionContext,
        _intent: &ToolIntent,
        _intent_sequence: usize,
        _request: &ToolCoreRequest,
        _outcome: &ToolCoreOutcome,
        _binding: ConversationRuntimeBinding<'_>,
    ) {
    }
}

pub struct NoopAppToolDispatcher;

#[async_trait]
impl AppToolDispatcher for NoopAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        _session_context: &SessionContext,
        request: ToolCoreRequest,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!("app_tool_not_implemented: {}", request.tool_name))
    }
}

pub enum ToolExecutionPreflight {
    Ready {
        request: ToolCoreRequest,
        trusted_internal_context: bool,
    },
    NeedsApproval(ApprovalRequirement),
}

impl ToolExecutionPreflight {
    pub(crate) fn ready(request: ToolCoreRequest) -> Self {
        Self::Ready {
            request,
            trusted_internal_context: false,
        }
    }
}

#[derive(Clone)]
pub struct DefaultAppToolDispatcher {
    pub(super) memory_config: SessionStoreConfig,
    pub(super) tool_config: ToolConfig,
    pub(super) app_config: Option<Arc<LoongConfig>>,
}

impl DefaultAppToolDispatcher {
    pub fn new(memory_config: SessionStoreConfig, tool_config: ToolConfig) -> Self {
        Self {
            memory_config,
            tool_config,
            app_config: None,
        }
    }

    pub fn with_config(memory_config: SessionStoreConfig, app_config: LoongConfig) -> Self {
        Self {
            memory_config,
            tool_config: app_config.tools.clone(),
            app_config: Some(Arc::new(app_config)),
        }
    }

    pub fn runtime() -> Self {
        Self::new(
            store::current_session_store_config().clone(),
            ToolConfig::default(),
        )
    }
}

pub(super) enum GovernedToolPreflight {
    Allowed,
    AllowedWithTrustedInternalContext(Value),
    NeedsApproval(ApprovalRequirement),
}
