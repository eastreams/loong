use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use loong_contracts::{KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{
    GovernedToolApprovalMode, LoongConfig, SessionVisibility, ToolConfig, ToolConsentMode,
};
use crate::context::KernelContext;
#[cfg(feature = "memory-sqlite")]
use crate::operator::approval_runtime::{GovernedToolApprovalRequest, OperatorApprovalRuntime};
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::resolve_delegate_child_contract;
#[cfg(feature = "memory-sqlite")]
use crate::operator::session_graph::OperatorSessionGraph;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewApprovalRequestRecord, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};
use crate::session::store::{self, SessionStoreConfig};
#[cfg(all(feature = "memory-sqlite", test))]
use crate::task_progress::TASK_PROGRESS_EVENT_KIND;
#[cfg(feature = "memory-sqlite")]
use crate::task_progress::resolve_canonical_task_id_for_session;
use crate::tools::runtime_events::{
    ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
};
use crate::tools::{
    ResolvedToolExecution, ToolApprovalMode, ToolDescriptor, ToolExecutionKind,
    ToolSchedulingClass, ToolView, delegate_child_tool_view_for_contract,
    governance_profile_for_descriptor, runtime_tool_view, runtime_tool_view_for_config,
    tool_catalog,
};
#[cfg(feature = "memory-sqlite")]
use crate::trust::{approval_required_trust_event, embed_trust_event_payload};

use super::autonomy_policy::{
    AUTONOMY_POLICY_SOURCE, AutonomyTurnBudgetState, PolicyDecision, PolicyDecisionInput,
    evaluate_policy, render_reason,
};
use super::runtime::{SessionContext, load_default_conversation_runtime};
use super::runtime_binding::ConversationRuntimeBinding;
use super::tool_result_compaction::compact_tool_result_payload_value;
use super::turn_observer::{ConversationTurnObserverHandle, ConversationTurnRuntimeEvent};

use super::ingress::{ConversationIngressContext, inject_internal_tool_ingress};
use super::tool_input_contract::detect_repairable_tool_request_issue;
use super::turn_shared::effective_followup_visible_tool_name;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub assistant_text: String,
    pub tool_intents: Vec<ToolIntent>,
    pub raw_meta: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool_name: String,
    pub args_json: serde_json::Value,
    pub source: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
}

struct AugmentedToolPayload {
    payload: serde_json::Value,
    trusted_internal_context: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecision {
    pub allow: bool,
    pub deny: bool,
    pub reason: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecisionKind {
    Allow,
    ApprovalRequired,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDecisionTelemetry {
    pub tool_name: String,
    pub decision_kind: ToolDecisionKind,
    pub allow: bool,
    pub deny: bool,
    pub reason: String,
    pub rule_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autonomy_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_action_class: Option<String>,
}

impl ToolDecisionTelemetry {
    fn allow(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::Allow,
            allow: true,
            deny: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    fn approval_required(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::ApprovalRequired,
            allow: false,
            deny: false,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    fn deny(
        tool_name: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            decision_kind: ToolDecisionKind::Deny,
            allow: false,
            deny: true,
            reason: reason.into(),
            rule_id: rule_id.into(),
            reason_code: None,
            policy_source: None,
            autonomy_profile: None,
            capability_action_class: None,
        }
    }

    fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }

    fn with_policy_source(mut self, policy_source: impl Into<String>) -> Self {
        self.policy_source = Some(policy_source.into());
        self
    }

    fn with_autonomy_profile(mut self, autonomy_profile: impl Into<String>) -> Self {
        self.autonomy_profile = Some(autonomy_profile.into());
        self
    }

    fn with_capability_action_class(mut self, capability_action_class: impl Into<String>) -> Self {
        self.capability_action_class = Some(capability_action_class.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolOutcomeTelemetry {
    pub tool_name: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub error_code: Option<String>,
    pub human_reason: Option<String>,
    pub audit_event_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirementKind {
    KernelContextRequired,
    GovernedTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequirement {
    pub kind: ApprovalRequirementKind,
    pub reason: String,
    pub rule_id: String,
    pub tool_name: Option<String>,
    pub approval_key: Option<String>,
    pub approval_request_id: Option<String>,
}

impl ApprovalRequirement {
    pub fn governed_tool(
        tool_name: impl Into<String>,
        approval_key: impl Into<String>,
        reason: impl Into<String>,
        rule_id: impl Into<String>,
        approval_request_id: Option<String>,
    ) -> Self {
        Self {
            kind: ApprovalRequirementKind::GovernedTool,
            reason: reason.into(),
            rule_id: rule_id.into(),
            tool_name: Some(tool_name.into()),
            approval_key: Some(approval_key.into()),
            approval_request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPreflightOutcome {
    Allow(ToolDecisionTelemetry),
    NeedsApproval {
        requirement: ApprovalRequirement,
        decision: ToolDecisionTelemetry,
    },
    Denied {
        failure: TurnFailure,
        decision: ToolDecisionTelemetry,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultEnvelope {
    pub status: String,
    pub tool: String,
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_semantics: Option<ToolResultPayloadSemantics>,
    pub payload_summary: String,
    pub payload_chars: usize,
    pub payload_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultPayloadSemantics {
    DiscoveryResult,
    ExternalSkillContext,
}

const TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 2048;
const MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 256;
const MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 64_000;
const TOOL_PREFLIGHT_ALLOW_RULE_ID: &str = "tool_preflight_allowed";
const AUTONOMY_POLICY_ALLOW_RULE_ID: &str = "autonomy_policy_allow";
const AUTONOMY_POLICY_ALLOW_REASON_CODE: &str = "autonomy_policy_allow";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnFailureKind {
    PolicyDenied,
    Retryable,
    NonRetryable,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFailure {
    pub kind: TurnFailureKind,
    pub code: String,
    pub reason: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "turn_failure_flag_is_false")]
    pub supports_discovery_recovery: bool,
}

impl TurnFailure {
    pub fn policy_denied(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::PolicyDenied,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
        }
    }

    pub fn policy_denied_with_discovery_recovery(
        code: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind: TurnFailureKind::PolicyDenied,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: true,
        }
    }

    pub fn retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Retryable,
            code: code.into(),
            reason: reason.into(),
            retryable: true,
            supports_discovery_recovery: false,
        }
    }

    pub fn non_retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::NonRetryable,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
        }
    }

    pub fn provider(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Provider,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
        }
    }

    pub fn as_str(&self) -> &str {
        self.reason.as_str()
    }
}

fn turn_failure_flag_is_false(value: &bool) -> bool {
    !*value
}

impl Deref for TurnFailure {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.reason.as_str()
    }
}

impl fmt::Display for TurnFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason.as_str())
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnResult {
    FinalText(String),
    StreamingText(String),
    StreamingDone(String),
    NeedsApproval(ApprovalRequirement),
    ToolDenied(TurnFailure),
    ToolError(TurnFailure),
    ProviderError(TurnFailure),
}

impl TurnResult {
    pub fn policy_denied(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ToolDenied(TurnFailure::policy_denied(code, reason))
    }

    pub fn retryable_tool_error(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ToolError(TurnFailure::retryable(code, reason))
    }

    pub fn non_retryable_tool_error(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ToolError(TurnFailure::non_retryable(code, reason))
    }

    pub fn provider_error(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ProviderError(TurnFailure::provider(code, reason))
    }

    pub fn failure(&self) -> Option<&TurnFailure> {
        match self {
            TurnResult::FinalText(_)
            | TurnResult::StreamingText(_)
            | TurnResult::StreamingDone(_)
            | TurnResult::NeedsApproval(_) => None,
            TurnResult::ToolDenied(failure)
            | TurnResult::ToolError(failure)
            | TurnResult::ProviderError(failure) => Some(failure),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnValidation {
    FinalText(String),
    ToolExecutionRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KernelFailureClass {
    PolicyDenied,
    RetryableExecution,
    NonRetryable,
}

pub(crate) fn classify_kernel_error(error: &KernelError) -> KernelFailureClass {
    #[allow(clippy::wildcard_enum_match_arm)]
    match error {
        KernelError::Policy(_)
        | KernelError::PackCapabilityBoundary { .. }
        | KernelError::ConnectorNotAllowed { .. } => KernelFailureClass::PolicyDenied,
        KernelError::ToolPlane(ToolPlaneError::Execution(reason)) => {
            classify_tool_execution_reason(reason)
        }
        _ => KernelFailureClass::NonRetryable,
    }
}

fn generic_allow_tool_decision(tool_name: &str) -> ToolDecisionTelemetry {
    let reason = format!("tool preflight allowed `{tool_name}`");
    ToolDecisionTelemetry::allow(tool_name, reason, TOOL_PREFLIGHT_ALLOW_RULE_ID)
}

fn approval_required_tool_decision(
    tool_name: &str,
    requirement: &ApprovalRequirement,
) -> ToolDecisionTelemetry {
    let reason = requirement.reason.clone();
    let rule_id = requirement.rule_id.clone();
    ToolDecisionTelemetry::approval_required(tool_name, reason, rule_id)
}

fn denied_tool_decision(tool_name: &str, failure: &TurnFailure) -> ToolDecisionTelemetry {
    let reason = failure.reason.clone();
    let rule_id = failure.code.clone();
    ToolDecisionTelemetry::deny(tool_name, reason, rule_id)
}

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
    fn ready(request: ToolCoreRequest) -> Self {
        Self::Ready {
            request,
            trusted_internal_context: false,
        }
    }
}

#[derive(Clone)]
pub struct DefaultAppToolDispatcher {
    memory_config: SessionStoreConfig,
    tool_config: ToolConfig,
    app_config: Option<Arc<LoongConfig>>,
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

    fn autonomy_policy_decision_base(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
    ) -> ToolDecisionTelemetry {
        let profile = policy_snapshot.profile.as_str();
        let action_class_name = action_class.as_str();
        let base = ToolDecisionTelemetry::allow(tool_name, "", AUTONOMY_POLICY_ALLOW_RULE_ID);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(profile);
        let with_action_class = with_profile.with_capability_action_class(action_class_name);
        with_action_class.with_reason_code(AUTONOMY_POLICY_ALLOW_REASON_CODE)
    }

    fn autonomy_policy_allow_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
    ) -> ToolDecisionTelemetry {
        let profile = policy_snapshot.profile.as_str();
        let reason =
            format!("autonomy policy allowed `{tool_name}` under `{profile}` product mode");
        let base = Self::autonomy_policy_decision_base(tool_name, policy_snapshot, action_class);
        ToolDecisionTelemetry { reason, ..base }
    }

    fn autonomy_policy_grant_satisfied_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = Self::autonomy_policy_decision_base(tool_name, policy_snapshot, action_class);
        let decision = ToolDecisionTelemetry {
            reason,
            rule_id: rule_id.to_owned(),
            ..base
        };
        decision.with_reason_code(reason_code)
    }

    fn autonomy_policy_approval_required_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = ToolDecisionTelemetry::approval_required(tool_name, reason, rule_id);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(policy_snapshot.profile.as_str());
        let with_action_class = with_profile.with_capability_action_class(action_class.as_str());
        with_action_class.with_reason_code(reason_code)
    }

    fn autonomy_policy_denied_decision(
        tool_name: &str,
        policy_snapshot: &crate::tools::runtime_config::AutonomyPolicySnapshot,
        action_class: crate::tools::CapabilityActionClass,
        rule_id: &str,
        reason_code: &str,
        reason: String,
    ) -> ToolDecisionTelemetry {
        let base = ToolDecisionTelemetry::deny(tool_name, reason, rule_id);
        let with_source = base.with_policy_source(AUTONOMY_POLICY_SOURCE);
        let with_profile = with_source.with_autonomy_profile(policy_snapshot.profile.as_str());
        let with_action_class = with_profile.with_capability_action_class(action_class.as_str());
        with_action_class.with_reason_code(reason_code)
    }

    fn effective_tool_config_for_session(&self, session_context: &SessionContext) -> ToolConfig {
        let mut tool_config = self.tool_config.clone();
        if session_context.parent_session_id.is_some() {
            tool_config.sessions.visibility = SessionVisibility::SelfOnly;
        }
        tool_config
    }

    #[cfg(feature = "memory-sqlite")]
    fn effective_tool_view_for_session(
        &self,
        session_context: &SessionContext,
    ) -> Result<ToolView, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        if let Some(session) = repo.load_session(&session_context.session_id)? {
            if session.parent_session_id.is_some() {
                let subagent_contract = match session_context.resolved_subagent_contract() {
                    Some(subagent_contract) => Some(subagent_contract),
                    None => resolve_delegate_child_contract(
                        &repo,
                        &session_context.session_id,
                        self.tool_config.delegate.max_depth,
                    )?,
                };
                return Ok(with_runtime_ready_browser_companion_tools(
                    delegate_child_tool_view_for_contract(
                        &self.tool_config,
                        subagent_contract.as_ref(),
                    ),
                    &session_context.tool_view,
                ));
            }
            return Ok(with_runtime_ready_browser_companion_tools(
                runtime_tool_view_for_config(&self.tool_config),
                &session_context.tool_view,
            ));
        }
        if repo
            .load_session_summary_with_legacy_fallback(&session_context.session_id)?
            .is_some_and(|session| session.kind == SessionKind::DelegateChild)
        {
            let subagent_contract = resolve_delegate_child_contract(
                &repo,
                &session_context.session_id,
                self.tool_config.delegate.max_depth,
            )?;
            return Ok(with_runtime_ready_browser_companion_tools(
                delegate_child_tool_view_for_contract(
                    &self.tool_config,
                    subagent_contract.as_ref(),
                ),
                &session_context.tool_view,
            ));
        }
        Ok(with_runtime_ready_browser_companion_tools(
            runtime_tool_view_for_config(&self.tool_config),
            &session_context.tool_view,
        ))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn effective_tool_view_for_session(
        &self,
        session_context: &SessionContext,
    ) -> Result<ToolView, String> {
        Ok(with_runtime_ready_browser_companion_tools(
            runtime_tool_view_for_config(&self.tool_config),
            &session_context.tool_view,
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    async fn execute_sessions_send(
        &self,
        session_context: &SessionContext,
        payload: serde_json::Value,
    ) -> Result<ToolCoreOutcome, String> {
        let app_config = self
            .app_config
            .as_ref()
            .ok_or_else(|| "sessions_send_not_configured".to_owned())?;
        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        crate::tools::messaging::execute_sessions_send_with_config(
            payload,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
            app_config.as_ref(),
        )
        .await
    }

    #[cfg(feature = "memory-sqlite")]
    fn lineage_root_session_id(
        repo: &SessionRepository,
        session_context: &SessionContext,
    ) -> Result<String, String> {
        let session_graph = OperatorSessionGraph::new(repo);
        session_graph.effective_lineage_root_session_id(
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
        )
    }

    fn autonomy_policy_snapshot(&self) -> crate::tools::runtime_config::AutonomyPolicySnapshot {
        crate::tools::runtime_config::AutonomyPolicySnapshot::from_profile(
            self.tool_config.autonomy_profile,
        )
    }

    fn approval_key_for_descriptor(descriptor: &crate::tools::ToolDescriptor) -> String {
        OperatorApprovalRuntime::approval_key_for_tool_name(descriptor.name)
    }

    fn is_tool_call_preapproved(&self, approval_key: &str) -> bool {
        let approved_calls = &self.tool_config.approval.approved_calls;
        approved_calls.iter().any(|entry| entry == approval_key)
    }

    fn is_tool_call_predenied(&self, approval_key: &str) -> bool {
        let denied_calls = &self.tool_config.approval.denied_calls;
        denied_calls.iter().any(|entry| entry == approval_key)
    }

    #[cfg(feature = "memory-sqlite")]
    fn approval_request_payload_json(
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        approval_request_id: &str,
        approval_key: &str,
        rule_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> serde_json::Value {
        let payload = json!({
            "session_id": session_context.session_id,
            "parent_session_id": session_context.parent_session_id,
            "turn_id": intent.turn_id,
            "tool_call_id": intent.tool_call_id,
            "tool_name": descriptor.name,
            "approval_key": approval_key,
            "approval_request_id": approval_request_id,
            "args_json": intent.args_json,
            "source": intent.source,
            "execution_kind": match descriptor.execution_kind {
                ToolExecutionKind::Core => "core",
                ToolExecutionKind::App => "app",
            },
        });
        let provenance_ref = approval_request_provenance_ref(binding);
        let trust_event = approval_required_trust_event(
            &session_context.session_id,
            "conversation.approval",
            provenance_ref,
            rule_id,
            Some(approval_request_id),
            Some(descriptor.name),
        );

        embed_trust_event_payload(payload, trust_event)
    }

    #[cfg(feature = "memory-sqlite")]
    fn persist_approval_request(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        approval_key: &str,
        reason: &str,
        rule_id: &str,
        governance_snapshot_json: serde_json::Value,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ApprovalRequirement, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let kind = if session_context.parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let _ = repo.ensure_session(NewSessionRecord {
            session_id: session_context.session_id.clone(),
            kind,
            parent_session_id: session_context.parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        })?;

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            descriptor,
            &approval_request_id,
            approval_key,
            rule_id,
            binding,
        );
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.to_owned(),
            request_payload_json,
            governance_snapshot_json,
        })?;

        Ok(ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        ))
    }

    #[cfg(feature = "memory-sqlite")]
    fn has_approval_grant(
        &self,
        session_context: &SessionContext,
        approval_key: &str,
    ) -> Result<bool, String> {
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let grant = approval_runtime.load_runtime_grant_for_context(
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
            approval_key,
        )?;
        Ok(grant.is_some())
    }

    #[cfg(feature = "memory-sqlite")]
    fn maybe_require_governed_tool_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.execution_kind != ToolExecutionKind::App
            || governance.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return Ok(None);
        }

        let requires_approval = match self.tool_config.approval.mode {
            GovernedToolApprovalMode::Disabled => false,
            GovernedToolApprovalMode::MediumBalanced => {
                governance.risk_class == crate::tools::ToolRiskClass::High
            }
            GovernedToolApprovalMode::Strict => true,
        };
        if !requires_approval {
            return Ok(None);
        }

        let approval_key = Self::approval_key_for_descriptor(descriptor);
        let is_preapproved = self.is_tool_call_preapproved(&approval_key);
        if is_preapproved {
            return Ok(None);
        }
        let is_predenied = self.is_tool_call_predenied(&approval_key);
        if is_predenied {
            return Err(format!(
                "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
            ));
        }
        let repo = SessionRepository::new(&self.memory_config)?;
        let approval_runtime = OperatorApprovalRuntime::new(&repo);
        let runtime_grant = approval_runtime.load_runtime_grant_for_context(
            &session_context.session_id,
            session_context.parent_session_id.as_deref(),
            &approval_key,
        )?;
        if runtime_grant.is_some() {
            return Ok(None);
        }

        let reason = format!(
            "operator approval required before running `{}`",
            descriptor.name
        );
        let rule_id = "governed_tool_requires_approval";
        let approval_request = GovernedToolApprovalRequest {
            session_id: &session_context.session_id,
            parent_session_id: session_context.parent_session_id.as_deref(),
            turn_id: &intent.turn_id,
            tool_call_id: &intent.tool_call_id,
            tool_name: descriptor.name,
            args_json: intent.args_json.clone(),
            source: &intent.source,
            governance_scope: governance.scope.as_str(),
            risk_class: governance.risk_class.as_str(),
            approval_mode: governance.approval_mode.as_str(),
            reason: &reason,
            rule_id,
            provenance_ref: approval_request_provenance_ref(binding),
        };
        let stored = approval_runtime.ensure_governed_tool_approval_request(approval_request)?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(Some(requirement))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn maybe_require_governed_tool_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let _ = (session_context, intent, descriptor, binding);
        Ok(None)
    }

    fn governed_tool_requires_operator_approval(
        &self,
        descriptor: &crate::tools::ToolDescriptor,
    ) -> bool {
        let governance = governance_profile_for_descriptor(descriptor);
        match self.tool_config.approval.mode {
            GovernedToolApprovalMode::Disabled => false,
            GovernedToolApprovalMode::MediumBalanced => {
                governance.risk_class == crate::tools::ToolRiskClass::High
            }
            GovernedToolApprovalMode::Strict => {
                governance.approval_mode == ToolApprovalMode::PolicyDriven
            }
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn ensure_governed_tool_session_scope(
        &self,
        repo: &SessionRepository,
        session_context: &SessionContext,
    ) -> Result<String, String> {
        let session_kind = if session_context.parent_session_id.is_some() {
            SessionKind::DelegateChild
        } else {
            SessionKind::Root
        };
        let session_record = NewSessionRecord {
            session_id: session_context.session_id.clone(),
            kind: session_kind,
            parent_session_id: session_context.parent_session_id.clone(),
            label: None,
            state: SessionState::Ready,
        };
        let _ = repo.ensure_session(session_record)?;
        Self::lineage_root_session_id(repo, session_context)
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_app_tool_preflight(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if descriptor.execution_kind != ToolExecutionKind::App
            || governance.approval_mode != ToolApprovalMode::PolicyDriven
        {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let approval_key = format!("tool:{}", descriptor.name);
        let approved_calls = &self.tool_config.approval.approved_calls;
        let approved_by_policy = approved_calls.iter().any(|entry| entry == &approval_key);
        if approved_by_policy {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let denied_calls = &self.tool_config.approval.denied_calls;
        let denied_by_policy = denied_calls.iter().any(|entry| entry == &approval_key);
        if denied_by_policy {
            let reason = format!(
                "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
            );
            return Err(reason);
        }

        let repo = SessionRepository::new(&self.memory_config)?;
        let scope_session_id = self.ensure_governed_tool_session_scope(&repo, session_context)?;
        let grant_record = repo.load_approval_grant(&scope_session_id, &approval_key)?;
        if grant_record.is_some() {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let reason = format!(
            "operator approval required before running `{}`",
            descriptor.name
        );
        let rule_id = "governed_tool_requires_approval";
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            descriptor,
            &approval_request_id,
            &approval_key,
            rule_id,
            binding,
        );
        let governance_snapshot_json = json!({
            "governance_scope": governance.scope.as_str(),
            "risk_class": governance.risk_class.as_str(),
            "approval_mode": governance.approval_mode.as_str(),
            "rule_id": rule_id,
            "reason": reason,
        });
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.clone(),
            request_payload_json,
            governance_snapshot_json,
        })?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(GovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_shell_tool_preflight(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let payload = request.payload.as_object();
        let Some(payload) = payload else {
            return Ok(GovernedToolPreflight::Allowed);
        };
        let command = payload.get("command").and_then(Value::as_str);
        let Some(command) = command else {
            return Ok(GovernedToolPreflight::Allowed);
        };
        let trimmed_command = command.trim();
        if trimmed_command.is_empty() {
            return Ok(GovernedToolPreflight::Allowed);
        }
        let normalized_command = crate::tools::shell_policy_ext::validate_shell_command_name(
            trimmed_command,
        )
        .map_err(|reason| {
            if crate::tools::shell_policy_ext::is_repairable_tool_input_reason(reason.as_str()) {
                let stripped = crate::tools::shell_policy_ext::strip_repairable_tool_input_prefix(
                    reason.as_str(),
                );
                return RepairableToolPreflight::encode(stripped);
            }
            format!("tool_preflight_denied: {reason}")
        })?;

        let shell_deny = &self.tool_config.shell_deny;
        let hard_denied = shell_deny
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&normalized_command));
        if hard_denied {
            let reason = format!(
                "tool_preflight_denied: shell command `{normalized_command}` is blocked by shell policy"
            );
            return Err(reason);
        }

        let shell_allow = &self.tool_config.shell_allow;
        let explicitly_allowed = shell_allow
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&normalized_command));
        let default_allows = self.tool_config.shell_default_mode == "allow";
        if explicitly_allowed || default_allows {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let requires_approval = self.governed_tool_requires_operator_approval(descriptor);
        if !requires_approval {
            return Ok(GovernedToolPreflight::Allowed);
        }

        let approval_key =
            crate::tools::shell_policy_ext::shell_exec_approval_key_for_normalized_command(
                normalized_command.as_str(),
            );
        let approved_calls = &self.tool_config.approval.approved_calls;
        let approved_by_policy = approved_calls.iter().any(|entry| entry == &approval_key);
        if approved_by_policy {
            let internal_context =
                crate::tools::shell_policy_ext::shell_exec_internal_approval_context(
                    approval_key.as_str(),
                );
            return Ok(GovernedToolPreflight::AllowedWithTrustedInternalContext(
                internal_context,
            ));
        }

        let denied_calls = &self.tool_config.approval.denied_calls;
        let denied_by_policy = denied_calls.iter().any(|entry| entry == &approval_key);
        if denied_by_policy {
            let reason = format!(
                "tool_preflight_denied: governed tool `{approval_key}` is denied by approval policy"
            );
            return Err(reason);
        }

        let repo = SessionRepository::new(&self.memory_config)?;
        let scope_session_id = self.ensure_governed_tool_session_scope(&repo, session_context)?;
        let grant_record = repo.load_approval_grant(&scope_session_id, &approval_key)?;
        if grant_record.is_some() {
            let internal_context =
                crate::tools::shell_policy_ext::shell_exec_internal_approval_context(
                    approval_key.as_str(),
                );
            return Ok(GovernedToolPreflight::AllowedWithTrustedInternalContext(
                internal_context,
            ));
        }

        let approval_request_id =
            governed_approval_request_id(session_context, descriptor.name, intent);
        let visible_tool_name = crate::tools::model_visible_tool_name(descriptor.name);
        let reason = format!(
            "operator approval required before running shell command `{normalized_command}` via `{visible_tool_name}`"
        );
        let rule_id = crate::tools::shell_policy_ext::SHELL_EXEC_APPROVAL_RULE_ID;
        let request_payload_json = Self::approval_request_payload_json(
            session_context,
            intent,
            descriptor,
            &approval_request_id,
            &approval_key,
            rule_id,
            binding,
        );
        let governance = governance_profile_for_descriptor(descriptor);
        let governance_snapshot_json = json!({
            "governance_scope": governance.scope.as_str(),
            "risk_class": governance.risk_class.as_str(),
            "approval_mode": governance.approval_mode.as_str(),
            "rule_id": rule_id,
            "reason": reason,
        });
        let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id,
            session_id: session_context.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
            tool_name: descriptor.name.to_owned(),
            approval_key: approval_key.clone(),
            request_payload_json,
            governance_snapshot_json,
        })?;
        let requirement = ApprovalRequirement::governed_tool(
            descriptor.name,
            approval_key,
            reason,
            rule_id,
            Some(stored.approval_request_id),
        );
        Ok(GovernedToolPreflight::NeedsApproval(requirement))
    }

    #[cfg(feature = "memory-sqlite")]
    fn governed_tool_preflight(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: &ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<GovernedToolPreflight, String> {
        let governance = governance_profile_for_descriptor(descriptor);
        if governance.approval_mode != ToolApprovalMode::PolicyDriven {
            return Ok(GovernedToolPreflight::Allowed);
        }

        if descriptor.name == crate::tools::SHELL_EXEC_TOOL_NAME {
            return self.governed_shell_tool_preflight(
                session_context,
                intent,
                request,
                descriptor,
                binding,
            );
        }

        self.governed_app_tool_preflight(session_context, intent, descriptor, binding)
    }
}

#[cfg(feature = "memory-sqlite")]
fn approval_request_provenance_ref(binding: ConversationRuntimeBinding<'_>) -> &'static str {
    if binding.is_kernel_bound() {
        return "kernel";
    }

    "advisory_only"
}

fn governed_approval_request_id(
    session_context: &SessionContext,
    tool_name: &str,
    intent: &ToolIntent,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_context.session_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.turn_id.as_bytes());
    hasher.update([0]);
    hasher.update(intent.tool_call_id.as_bytes());
    hasher.update([0]);
    hasher.update(tool_name.as_bytes());
    format!("apr_{}", hex::encode(hasher.finalize()))
}

fn tool_is_session_consent_exempt(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "approval_request_resolve" | "approval_request_status" | "approval_requests_list"
    )
}

fn tool_intent_skips_provider_exposed_gate(
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> bool {
    if descriptor.name == "tool.invoke" {
        return true;
    }

    intent.source == "approval_control" && tool_is_session_consent_exempt(descriptor.name)
}

fn tool_is_auto_eligible(
    descriptor: &crate::tools::ToolDescriptor,
    governance: crate::tools::ToolGovernanceProfile,
) -> bool {
    tool_is_session_consent_exempt(descriptor.name)
        || (governance.risk_class == crate::tools::ToolRiskClass::Low
            && governance.approval_mode == ToolApprovalMode::Never)
}

enum GovernedToolPreflight {
    Allowed,
    AllowedWithTrustedInternalContext(Value),
    NeedsApproval(ApprovalRequirement),
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
    fn memory_config(&self) -> Option<&SessionStoreConfig> {
        Some(&self.memory_config)
    }

    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &AutonomyTurnBudgetState,
    ) -> Result<ToolPreflightOutcome, String> {
        let policy_snapshot = self.autonomy_policy_snapshot();
        let action_class = descriptor.capability_action_class();
        let policy_input = PolicyDecisionInput {
            snapshot: &policy_snapshot,
            action_class,
            binding,
            budget: budget_state,
        };
        let autonomy_policy_applies =
            super::autonomy_policy::action_mode(&policy_snapshot, action_class).is_some();
        let policy_decision = evaluate_policy(policy_input);
        let mut autonomy_allow_decision = None;
        match policy_decision {
            PolicyDecision::Allow => {
                if autonomy_policy_applies {
                    let decision = Self::autonomy_policy_allow_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                    );
                    autonomy_allow_decision = Some(decision);
                }
            }
            PolicyDecision::ApprovalRequired {
                rule_id,
                reason_code,
            } => {
                let reason =
                    render_reason(&policy_snapshot, action_class, descriptor.name, reason_code);
                let approval_key = Self::approval_key_for_descriptor(descriptor);

                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, intent, approval_key);
                    let failure = TurnFailure::policy_denied(
                        "autonomy_policy_approval_support_missing",
                        reason.clone(),
                    );
                    let decision = Self::autonomy_policy_denied_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                        rule_id,
                        reason_code,
                        reason,
                    );
                    return Ok(ToolPreflightOutcome::Denied { failure, decision });
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let is_preapproved = self.is_tool_call_preapproved(&approval_key);
                    let is_predenied = self.is_tool_call_predenied(&approval_key);
                    if is_predenied {
                        let reason =
                            format!("governed tool `{approval_key}` is denied by approval policy");
                        let failure = TurnFailure::policy_denied("app_tool_denied", reason);
                        let decision = denied_tool_decision(descriptor.name, &failure);
                        return Ok(ToolPreflightOutcome::Denied { failure, decision });
                    }

                    let has_approval_grant =
                        self.has_approval_grant(session_context, approval_key.as_str())?;
                    let autonomy_approval_is_satisfied = is_preapproved || has_approval_grant;
                    if !autonomy_approval_is_satisfied {
                        let governance_snapshot_json = json!({
                            "policy_source": AUTONOMY_POLICY_SOURCE,
                            "decision_kind": ToolDecisionKind::ApprovalRequired,
                            "autonomy_profile": policy_snapshot.profile.as_str(),
                            "capability_action_class": action_class.as_str(),
                            "rule_id": rule_id,
                            "reason_code": reason_code,
                            "reason": reason,
                        });
                        let requirement = self.persist_approval_request(
                            session_context,
                            intent,
                            descriptor,
                            approval_key.as_str(),
                            reason.as_str(),
                            rule_id,
                            governance_snapshot_json,
                            binding,
                        )?;
                        let decision = Self::autonomy_policy_approval_required_decision(
                            descriptor.name,
                            &policy_snapshot,
                            action_class,
                            rule_id,
                            reason_code,
                            reason,
                        );
                        return Ok(ToolPreflightOutcome::NeedsApproval {
                            requirement,
                            decision,
                        });
                    }

                    let satisfied_reason = if is_preapproved {
                        format!(
                            "configured approval policy already allows `{}` under `{}` product mode",
                            descriptor.name,
                            policy_snapshot.profile.as_str()
                        )
                    } else {
                        format!(
                            "stored approval grant satisfied `{}` under `{}` product mode",
                            descriptor.name,
                            policy_snapshot.profile.as_str()
                        )
                    };
                    let decision = Self::autonomy_policy_grant_satisfied_decision(
                        descriptor.name,
                        &policy_snapshot,
                        action_class,
                        rule_id,
                        reason_code,
                        satisfied_reason,
                    );
                    autonomy_allow_decision = Some(decision);
                }
            }
            PolicyDecision::Deny {
                rule_id,
                reason_code,
            } => {
                let reason =
                    render_reason(&policy_snapshot, action_class, descriptor.name, reason_code);
                let failure = TurnFailure::policy_denied(reason_code, reason.clone());
                let decision = Self::autonomy_policy_denied_decision(
                    descriptor.name,
                    &policy_snapshot,
                    action_class,
                    rule_id,
                    reason_code,
                    reason,
                );
                return Ok(ToolPreflightOutcome::Denied { failure, decision });
            }
        }

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
                let decision = autonomy_allow_decision
                    .unwrap_or_else(|| generic_allow_tool_decision(descriptor.name));
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
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor, binding);
            Ok(None)
        }

        #[cfg(feature = "memory-sqlite")]
        {
            let _ = binding;
            let governance = governance_profile_for_descriptor(descriptor);
            let approval_key = Self::approval_key_for_descriptor(descriptor);
            let governed_approval_eligible = descriptor.execution_kind == ToolExecutionKind::App
                && governance.approval_mode == ToolApprovalMode::PolicyDriven;
            let approval_key_is_denied = governed_approval_eligible
                && self
                    .tool_config
                    .approval
                    .denied_calls
                    .iter()
                    .any(|entry| entry == &approval_key);

            if approval_key_is_denied {
                return Err(format!(
                    "app_tool_denied: governed tool `{approval_key}` is denied by approval policy"
                ));
            }

            let repo = SessionRepository::new(&self.memory_config)?;
            let kind = if session_context.parent_session_id.is_some() {
                SessionKind::DelegateChild
            } else {
                SessionKind::Root
            };
            let _ = repo.ensure_session(NewSessionRecord {
                session_id: session_context.session_id.clone(),
                kind,
                parent_session_id: session_context.parent_session_id.clone(),
                label: None,
                state: SessionState::Ready,
            })?;

            let scope_session_id = Self::lineage_root_session_id(&repo, session_context)?;
            let session_consent_mode = repo
                .load_session_tool_consent(&scope_session_id)?
                .map(|record| record.mode)
                .unwrap_or(self.tool_config.consent.default_mode);

            let session_consent_requirement = if tool_is_session_consent_exempt(descriptor.name) {
                None
            } else {
                match session_consent_mode {
                    ToolConsentMode::Prompt => Some((
                        "session_tool_consent_prompt_mode",
                        format!(
                            "session confirmation required before running `{}`",
                            descriptor.name
                        ),
                    )),
                    ToolConsentMode::Auto if !tool_is_auto_eligible(descriptor, governance) => {
                        Some((
                            "session_tool_consent_auto_blocked",
                            format!(
                                "`{}` is not eligible for auto mode and needs operator confirmation",
                                descriptor.name
                            ),
                        ))
                    }
                    ToolConsentMode::Auto | ToolConsentMode::Full => None,
                }
            };
            let Some((rule_id, reason)) = session_consent_requirement else {
                return self.maybe_require_governed_tool_approval_with_binding(
                    session_context,
                    intent,
                    descriptor,
                    binding,
                );
            };

            let governance_snapshot_json = json!({
                "governance_scope": governance.scope.as_str(),
                "risk_class": governance.risk_class.as_str(),
                "approval_mode": governance.approval_mode.as_str(),
                "session_consent_mode": session_consent_mode.as_str(),
                "rule_id": rule_id,
                "reason": reason,
            });
            let requirement = self.persist_approval_request(
                session_context,
                intent,
                descriptor,
                approval_key.as_str(),
                reason.as_str(),
                rule_id,
                governance_snapshot_json,
                binding,
            )?;

            Ok(Some(requirement))
        }
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        let repairable_issue = detect_repairable_tool_request_issue(descriptor, &request);

        if let Some(repairable_issue) = repairable_issue {
            let repairable_reason = repairable_issue.reason(descriptor.name);
            let encoded_reason = RepairableToolPreflight::encode(repairable_reason.as_str());
            return Err(encoded_reason);
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor, binding);
            Ok(ToolExecutionPreflight::ready(request))
        }

        #[cfg(feature = "memory-sqlite")]
        {
            if descriptor.name != crate::tools::SHELL_EXEC_TOOL_NAME {
                return Ok(ToolExecutionPreflight::ready(request));
            }

            let preflight = self.governed_tool_preflight(
                session_context,
                intent,
                &request,
                descriptor,
                binding,
            )?;
            match preflight {
                GovernedToolPreflight::Allowed => Ok(ToolExecutionPreflight::ready(request)),
                GovernedToolPreflight::NeedsApproval(requirement) => {
                    Ok(ToolExecutionPreflight::NeedsApproval(requirement))
                }
                GovernedToolPreflight::AllowedWithTrustedInternalContext(internal_context) => {
                    let mut request = request;
                    let payload = request.payload.as_object_mut().ok_or_else(|| {
                        format!(
                            "tool_preflight_invalid_payload: `{}` payload must be an object",
                            descriptor.name
                        )
                    })?;
                    crate::tools::merge_trusted_internal_tool_context_into_arguments(
                        payload,
                        &internal_context,
                    )?;
                    Ok(ToolExecutionPreflight::Ready {
                        request,
                        trusted_internal_context: true,
                    })
                }
            }
        }
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = self.effective_tool_view_for_session(session_context)?;
        let descriptor = tool_catalog().descriptor(canonical_tool_name);
        let has_kernel_context = binding.kernel_context().is_some();

        if let Some(descriptor) = descriptor
            && descriptor.execution_kind == ToolExecutionKind::App
            && (!session_context.tool_view.contains(descriptor.name)
                || !effective_tool_view.contains(descriptor.name))
        {
            return Err(format!("tool_not_visible: {}", descriptor.name));
        }

        let requires_kernel_binding = descriptor
            .map(crate::tools::ToolDescriptor::requires_kernel_binding)
            .unwrap_or(false);
        let effective_tool_config = self.effective_tool_config_for_session(session_context);

        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "session_continue" {
            let app_config = self
                .app_config
                .as_ref()
                .ok_or_else(|| "session_continue_not_configured".to_owned())?;
            let runtime = load_default_conversation_runtime(app_config.as_ref())?;
            return crate::tools::continue_session_with_runtime(
                request.payload,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
                app_config.as_ref(),
                &runtime,
                binding,
            )
            .await;
        }

        if requires_kernel_binding && !has_kernel_context {
            return Err("app_tool_denied: no_kernel_context".to_owned());
        }

        if canonical_tool_name == "session_wait" {
            return crate::tools::wait_for_session_with_config(
                request.payload,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        if canonical_tool_name == "task_wait" {
            return crate::tools::wait_for_task_with_config(
                request.payload,
                &session_context.session_id,
                &self.memory_config,
                &effective_tool_config,
            )
            .await;
        }
        #[cfg(feature = "memory-sqlite")]
        if canonical_tool_name == "sessions_send" {
            return self
                .execute_sessions_send(session_context, request.payload)
                .await;
        }
        crate::tools::execute_app_tool_with_visibility_checked_config(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
        )
    }
}

fn classify_tool_execution_reason(reason: &str) -> KernelFailureClass {
    if reason.starts_with("policy_denied: ") {
        KernelFailureClass::PolicyDenied
    } else {
        KernelFailureClass::RetryableExecution
    }
}

struct RepairableToolPreflight;

impl RepairableToolPreflight {
    const PREFIX: &str = "tool_preflight_repairable: ";

    fn encode(reason: &str) -> String {
        format!("{}{reason}", Self::PREFIX)
    }

    fn parse(encoded: &str) -> Option<&str> {
        encoded.strip_prefix(Self::PREFIX)
    }

    fn render(reason: &str) -> String {
        format!("tool_preflight_denied: tool input needs repair: {reason}")
    }
}

fn render_app_tool_denied_reason(reason: &str) -> String {
    reason
        .strip_prefix("app_tool_denied: ")
        .unwrap_or(reason)
        .to_owned()
}

fn with_runtime_ready_browser_companion_tools(
    base_view: ToolView,
    session_tool_view: &ToolView,
) -> ToolView {
    let mut names: BTreeSet<String> = base_view.tool_names().map(str::to_owned).collect();
    names.extend(
        session_tool_view
            .tool_names()
            .filter(|name| name.starts_with("browser.companion."))
            .map(str::to_owned),
    );
    ToolView::from_tool_names(names)
}

pub(crate) fn render_kernel_error_reason(error: &KernelError) -> String {
    #[allow(clippy::wildcard_enum_match_arm)]
    match error {
        KernelError::ToolPlane(ToolPlaneError::Execution(reason)) => format!(
            "tool execution failed: {}",
            reason.strip_prefix("policy_denied: ").unwrap_or(reason)
        ),
        _ => format!("{error}"),
    }
}

fn augment_tool_payload_for_kernel(
    canonical_tool_name: &str,
    payload: serde_json::Value,
    session_context: &SessionContext,
    memory_config: &SessionStoreConfig,
) -> AugmentedToolPayload {
    let invoked_tool_name = if canonical_tool_name == "tool.invoke" {
        payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .map(str::to_owned)
    } else {
        None
    };
    let tool_search_context_name = if invoked_tool_name.as_deref() == Some("tool.search") {
        "tool.search"
    } else {
        canonical_tool_name
    };
    let augmented_tool_search = inject_tool_search_visibility_context_trusted(
        tool_search_context_name,
        payload,
        session_context,
        false,
    );
    let payload_after_tool_search = augmented_tool_search.payload;
    let tool_search_trusted = augmented_tool_search.trusted_internal_context;
    let augmented_runtime_narrowing = inject_runtime_narrowing_context_trusted(
        payload_after_tool_search,
        session_context,
        tool_search_trusted,
    );
    let payload_after_runtime_narrowing = augmented_runtime_narrowing.payload;
    let runtime_narrowing_trusted = augmented_runtime_narrowing.trusted_internal_context;
    let augmented_active_skill_workspace_root = inject_active_skill_workspace_root_context_trusted(
        canonical_tool_name,
        &invoked_tool_name,
        payload_after_runtime_narrowing,
        session_context,
        runtime_narrowing_trusted,
    );
    let payload_after_active_skill_workspace_root = augmented_active_skill_workspace_root.payload;
    let active_skill_workspace_root_trusted =
        augmented_active_skill_workspace_root.trusted_internal_context;
    let augmented_workspace_root = inject_workspace_root_context_trusted(
        payload_after_active_skill_workspace_root,
        session_context,
        active_skill_workspace_root_trusted,
    );
    let mut payload = augmented_workspace_root.payload;
    let trusted_internal_context = augmented_workspace_root.trusted_internal_context;
    let canonical_task_id = resolve_canonical_task_id_for_runtime(session_context, memory_config);

    if task_scope_injection_required(canonical_tool_name) {
        payload = inject_task_scope_field(payload, canonical_task_id.as_str());
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    // Direct browser tool calls: inject scope at the top level.
    if browser_scope_injection_required(canonical_tool_name) {
        payload = inject_browser_scope_field(payload, &session_context.session_id);
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    // tool.invoke wrapping a browser tool: inject scope into the nested arguments.
    let is_browser_invoke = invoked_tool_name
        .as_deref()
        .is_some_and(browser_scope_injection_required);
    if is_browser_invoke && let serde_json::Value::Object(mut outer) = payload {
        if let Some(arguments) = outer.remove("arguments") {
            outer.insert(
                "arguments".to_owned(),
                inject_browser_scope_field(arguments, &session_context.session_id),
            );
        }
        payload = serde_json::Value::Object(outer);
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    let is_task_invoke = invoked_tool_name
        .as_deref()
        .is_some_and(task_scope_injection_required);
    if is_task_invoke && let serde_json::Value::Object(mut outer) = payload {
        if let Some(arguments) = outer.remove("arguments") {
            outer.insert(
                "arguments".to_owned(),
                inject_task_scope_field(arguments, canonical_task_id.as_str()),
            );
        }
        payload = serde_json::Value::Object(outer);
        return AugmentedToolPayload {
            payload,
            trusted_internal_context,
        };
    }

    AugmentedToolPayload {
        payload,
        trusted_internal_context,
    }
}

fn inject_active_skill_workspace_root_context_trusted(
    canonical_tool_name: &str,
    invoked_tool_name: &Option<String>,
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let workspace_root = active_skill_workspace_root_for_tool_payload(
        canonical_tool_name,
        invoked_tool_name,
        &payload,
        session_context,
    )
    .or_else(|| {
        visible_skill_workspace_root_for_tool_payload(
            canonical_tool_name,
            invoked_tool_name,
            &payload,
            session_context,
        )
    });
    let Some(workspace_root) = workspace_root else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };

    inject_workspace_root_path_context_trusted(
        payload,
        &workspace_root,
        preserve_existing_internal_context,
    )
}

fn active_skill_workspace_root_for_tool_payload(
    canonical_tool_name: &str,
    invoked_tool_name: &Option<String>,
    payload: &serde_json::Value,
    session_context: &SessionContext,
) -> Option<std::path::PathBuf> {
    if session_context.active_external_skill_roots.is_empty() {
        return None;
    }

    let target_tool_name = invoked_tool_name.as_deref().unwrap_or(canonical_tool_name);
    if !matches!(
        target_tool_name,
        "file.read" | "glob.search" | "content.search"
    ) {
        return None;
    }

    let requested_path = if canonical_tool_name == "tool.invoke" {
        requested_file_tool_path("tool.invoke", payload)?
    } else {
        requested_file_tool_path(target_tool_name, payload)?
    };
    if requested_path.is_absolute() {
        let normalized_requested_path = if requested_path.exists() {
            std::fs::canonicalize(&requested_path).unwrap_or(requested_path)
        } else {
            requested_path
        };

        return session_context
            .active_external_skill_roots
            .iter()
            .find(|root| normalized_requested_path.starts_with(root))
            .cloned();
    }

    resolve_active_skill_root_for_relative_path(
        &session_context.active_external_skill_roots,
        requested_path.as_path(),
    )
}

fn visible_skill_workspace_root_for_tool_payload(
    canonical_tool_name: &str,
    invoked_tool_name: &Option<String>,
    payload: &serde_json::Value,
    session_context: &SessionContext,
) -> Option<std::path::PathBuf> {
    if session_context.visible_external_skill_roots.is_empty() {
        return None;
    }

    let target_tool_name = invoked_tool_name.as_deref().unwrap_or(canonical_tool_name);
    if !matches!(
        target_tool_name,
        "file.read" | "glob.search" | "content.search"
    ) {
        return None;
    }

    let requested_path = if canonical_tool_name == "tool.invoke" {
        requested_file_tool_path("tool.invoke", payload)?
    } else {
        requested_file_tool_path(target_tool_name, payload)?
    };
    if !requested_path.is_absolute() {
        return None;
    }

    let normalized_requested_path = if requested_path.exists() {
        std::fs::canonicalize(&requested_path).unwrap_or(requested_path)
    } else {
        requested_path
    };

    session_context
        .visible_external_skill_roots
        .iter()
        .find(|root| normalized_requested_path.starts_with(root))
        .cloned()
}

fn resolve_active_skill_root_for_relative_path(
    active_skill_roots: &[std::path::PathBuf],
    requested_path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let mut matches = active_skill_roots
        .iter()
        .filter_map(|root| {
            let candidate = root.join(requested_path);
            candidate.exists().then(|| root.clone())
        })
        .collect::<Vec<_>>();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn requested_file_tool_path(
    tool_name: &str,
    payload: &serde_json::Value,
) -> Option<std::path::PathBuf> {
    let payload_object = payload.as_object()?;
    match tool_name {
        "file.read" => payload_object
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from),
        "glob.search" | "content.search" => payload_object
            .get("root")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from),
        "tool.invoke" => {
            let inner_tool_name = payload_object
                .get("tool_id")
                .and_then(serde_json::Value::as_str)
                .map(crate::tools::canonical_tool_name)?;
            let arguments = payload_object.get("arguments")?;
            requested_file_tool_path(inner_tool_name, arguments)
        }
        _ => None,
    }
}

fn task_scope_injection_required(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "task_status" | "task_wait" | "task_history" | "task_events"
    )
}

fn inject_task_scope_field(payload: serde_json::Value, task_id: &str) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut object) => {
            let has_task_id = object
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            if !has_task_id {
                object.insert("task_id".to_owned(), json!(task_id));
            }
            serde_json::Value::Object(object)
        }
        other @ serde_json::Value::Null
        | other @ serde_json::Value::Bool(_)
        | other @ serde_json::Value::Number(_)
        | other @ serde_json::Value::String(_)
        | other @ serde_json::Value::Array(_) => other,
    }
}

fn resolve_canonical_task_id_for_runtime(
    session_context: &SessionContext,
    memory_config: &SessionStoreConfig,
) -> String {
    #[cfg(feature = "memory-sqlite")]
    {
        if let Ok(repo) = SessionRepository::new(memory_config) {
            let task_id = resolve_canonical_task_id_for_session(&repo, &session_context.session_id);
            if let Some(task_id) = task_id {
                return task_id;
            }
        }
    }

    session_context.session_id.clone()
}

fn inject_tool_search_visibility_context_trusted(
    canonical_tool_name: &str,
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    if canonical_tool_name != "tool.search" {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    }

    let serde_json::Value::Object(mut object) = payload else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };

    let mut internal = if preserve_existing_internal_context {
        crate::tools::take_trusted_internal_tool_context(&mut object)
    } else {
        serde_json::Map::new()
    };
    let mut tool_search_context = internal
        .remove(crate::tools::LOONG_INTERNAL_TOOL_SEARCH_KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let visible_tool_ids = session_context
        .tool_view
        .tool_names()
        .map(|tool_name| serde_json::Value::String(tool_name.to_owned()))
        .collect::<Vec<_>>();
    tool_search_context.insert(
        crate::tools::LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY.to_owned(),
        serde_json::Value::Array(visible_tool_ids),
    );
    internal.insert(
        crate::tools::LOONG_INTERNAL_TOOL_SEARCH_KEY.to_owned(),
        serde_json::Value::Object(tool_search_context),
    );
    object.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    AugmentedToolPayload {
        payload: serde_json::Value::Object(object),
        trusted_internal_context: true,
    }
}

fn inject_runtime_narrowing_context_trusted(
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let resolved_runtime_narrowing = session_context.resolved_runtime_narrowing();
    let Some(runtime_narrowing) = resolved_runtime_narrowing else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    if runtime_narrowing.is_empty() {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    }

    let serde_json::Value::Object(mut object) = payload else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    let mut internal = if preserve_existing_internal_context {
        crate::tools::take_trusted_internal_tool_context(&mut object)
    } else {
        serde_json::Map::new()
    };
    internal.insert(
        crate::tools::LOONG_INTERNAL_RUNTIME_NARROWING_KEY.to_owned(),
        serde_json::to_value(runtime_narrowing)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
    );
    object.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    AugmentedToolPayload {
        payload: serde_json::Value::Object(object),
        trusted_internal_context: true,
    }
}

fn inject_workspace_root_context_trusted(
    payload: serde_json::Value,
    session_context: &SessionContext,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let Some(workspace_root) = session_context.workspace_root.as_ref() else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };

    inject_workspace_root_path_context_trusted(
        payload,
        workspace_root.as_path(),
        preserve_existing_internal_context,
    )
}

fn inject_workspace_root_path_context_trusted(
    payload: serde_json::Value,
    workspace_root: &std::path::Path,
    preserve_existing_internal_context: bool,
) -> AugmentedToolPayload {
    let serde_json::Value::Object(mut object) = payload else {
        return AugmentedToolPayload {
            payload,
            trusted_internal_context: preserve_existing_internal_context,
        };
    };
    let mut internal = if preserve_existing_internal_context {
        crate::tools::take_trusted_internal_tool_context(&mut object)
    } else {
        serde_json::Map::new()
    };
    if preserve_existing_internal_context
        && internal.contains_key(crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY)
    {
        object.insert(
            crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
            serde_json::Value::Object(internal),
        );
        return AugmentedToolPayload {
            payload: serde_json::Value::Object(object),
            trusted_internal_context: true,
        };
    }
    let workspace_root_string = workspace_root.display().to_string();
    internal.insert(
        crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY.to_owned(),
        serde_json::Value::String(workspace_root_string),
    );
    object.insert(
        crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    AugmentedToolPayload {
        payload: serde_json::Value::Object(object),
        trusted_internal_context: true,
    }
}

fn browser_scope_injection_required(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "browser.open"
            | "browser.extract"
            | "browser.click"
            | "browser.companion.session.start"
            | "browser.companion.navigate"
            | "browser.companion.snapshot"
            | "browser.companion.wait"
            | "browser.companion.session.stop"
            | "browser.companion.click"
            | "browser.companion.type"
    )
}

fn inject_browser_scope_field(payload: serde_json::Value, session_id: &str) -> serde_json::Value {
    match payload {
        serde_json::Value::Object(mut object) => {
            object.insert(
                crate::tools::BROWSER_SESSION_SCOPE_FIELD.to_owned(),
                json!(session_id),
            );
            serde_json::Value::Object(object)
        }
        other @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)
        | serde_json::Value::Array(_)) => other,
    }
}

fn turn_result_from_tool_execution_failure(failure: TurnFailure) -> TurnResult {
    match failure.kind {
        TurnFailureKind::PolicyDenied => TurnResult::ToolDenied(failure),
        TurnFailureKind::Retryable | TurnFailureKind::NonRetryable => {
            TurnResult::ToolError(failure)
        }
        TurnFailureKind::Provider => TurnResult::ProviderError(failure),
    }
}

pub(crate) fn format_tool_result_line_with_limit(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
    payload_summary_limit_chars: usize,
) -> String {
    let envelope = build_tool_result_envelope(intent, outcome, payload_summary_limit_chars);
    let effective_tool_name = effective_result_tool_name(intent);
    let encoded = serde_json::to_string(&envelope).unwrap_or_else(|_| {
        format!(
            "{{\"status\":\"{}\",\"tool\":\"{}\",\"tool_call_id\":\"{}\",\"payload_summary\":\"[tool_payload_unserializable]\",\"payload_chars\":0,\"payload_truncated\":false}}",
            outcome.status,
            effective_tool_name,
            intent.tool_call_id
        )
    });
    format!("[{}] {encoded}", outcome.status)
}

fn build_tool_result_envelope(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
    payload_summary_limit_chars: usize,
) -> ToolResultEnvelope {
    let effective_tool_name = effective_result_tool_name(intent);
    let payload_semantics = detect_tool_result_payload_semantics(&outcome.payload);
    let normalized_limit = payload_summary_limit_chars.clamp(
        MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    );
    let compacted_payload =
        compact_tool_result_payload_value(effective_tool_name.as_str(), &outcome.payload);
    let payload_text = serde_json::to_string(&compacted_payload)
        .unwrap_or_else(|_| "[tool_payload_unserializable]".to_owned());
    let (payload_summary, payload_chars, payload_truncated) =
        summarize_tool_result_payload(payload_text.as_str(), payload_semantics, normalized_limit);

    ToolResultEnvelope {
        status: outcome.status.clone(),
        tool: effective_tool_name,
        tool_call_id: intent.tool_call_id.clone(),
        payload_semantics,
        payload_summary,
        payload_chars,
        payload_truncated,
    }
}

fn summarize_tool_result_payload(
    payload_text: &str,
    payload_semantics: Option<ToolResultPayloadSemantics>,
    payload_summary_limit_chars: usize,
) -> (String, usize, bool) {
    if payload_semantics.is_some() {
        let payload_chars = payload_text.chars().count();
        let payload_summary = payload_text.to_owned();
        return (payload_summary, payload_chars, false);
    }

    truncate_by_chars(payload_text, payload_summary_limit_chars)
}

fn detect_tool_result_payload_semantics(
    payload: &serde_json::Value,
) -> Option<ToolResultPayloadSemantics> {
    let looks_like_discovery_result = payload_looks_like_discovery_result(payload);
    if looks_like_discovery_result {
        return Some(ToolResultPayloadSemantics::DiscoveryResult);
    }

    let looks_like_external_skill_context = payload_looks_like_external_skill_context(payload);
    if looks_like_external_skill_context {
        return Some(ToolResultPayloadSemantics::ExternalSkillContext);
    }

    None
}

fn payload_looks_like_discovery_result(payload: &serde_json::Value) -> bool {
    let Some(payload_object) = payload.as_object() else {
        return false;
    };
    let Some(results) = payload_object
        .get("results")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };

    if results.is_empty() {
        return payload_object.contains_key("query");
    }

    results.iter().any(|result| {
        let Some(result_object) = result.as_object() else {
            return false;
        };
        result_object
            .get("tool_id")
            .and_then(serde_json::Value::as_str)
            .is_some()
            && result_object
                .get("lease")
                .and_then(serde_json::Value::as_str)
                .is_some()
    })
}

fn payload_looks_like_external_skill_context(payload: &serde_json::Value) -> bool {
    let Some(payload_object) = payload.as_object() else {
        return false;
    };
    payload_object
        .get("skill_id")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && payload_object
            .get("display_name")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && payload_object
            .get("instructions")
            .and_then(serde_json::Value::as_str)
            .is_some()
}

pub(crate) fn effective_result_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    let effective_canonical_tool_name = if canonical_tool_name != "tool.invoke" {
        canonical_tool_name
    } else if let Some((tool_name, _arguments)) =
        crate::tools::invoked_discoverable_tool_request(&intent.args_json)
    {
        tool_name
    } else {
        intent
            .args_json
            .get("tool_id")
            .and_then(serde_json::Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .unwrap_or(canonical_tool_name)
    };

    crate::tools::user_visible_tool_name(effective_canonical_tool_name)
}

fn effective_denied_tool_name(intent: &ToolIntent) -> String {
    effective_followup_visible_tool_name(intent)
}

fn build_tool_decision_trace_record(
    intent: &ToolIntent,
    decision: ToolDecisionTelemetry,
) -> ToolDecisionTraceRecord {
    ToolDecisionTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        decision,
    }
}

fn build_success_tool_outcome_trace_record(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> ToolOutcomeTraceRecord {
    let tool_name = effective_result_tool_name(intent);
    let outcome = ToolOutcomeTelemetry {
        tool_name,
        status: outcome.status.clone(),
        payload: build_bounded_tool_outcome_payload(intent, outcome),
        error_code: None,
        human_reason: None,
        audit_event_id: None,
    };
    ToolOutcomeTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        outcome,
    }
}

fn build_bounded_tool_outcome_payload(
    _intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> serde_json::Value {
    let payload_semantics = detect_tool_result_payload_semantics(&outcome.payload);
    let normalized_limit = TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS.clamp(
        MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    );
    let payload_text = serde_json::to_string(&outcome.payload)
        .unwrap_or_else(|_| "[tool_payload_unserializable]".to_owned());
    let (payload_summary, payload_chars, payload_truncated) =
        summarize_tool_result_payload(payload_text.as_str(), payload_semantics, normalized_limit);

    if !payload_truncated {
        return outcome.payload.clone();
    }

    json!({
        "payload_summary": payload_summary,
        "payload_chars": payload_chars,
        "payload_truncated": true,
    })
}

fn build_failure_tool_outcome_trace_record(
    intent: &ToolIntent,
    turn_result: &TurnResult,
) -> Option<ToolOutcomeTraceRecord> {
    let failure = turn_result.failure()?;
    let tool_name = effective_result_tool_name(intent);
    let outcome = ToolOutcomeTelemetry {
        tool_name,
        status: "error".to_owned(),
        payload: serde_json::Value::Null,
        error_code: Some(failure.code.clone()),
        human_reason: Some(failure.reason.clone()),
        audit_event_id: None,
    };
    Some(ToolOutcomeTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        outcome,
    })
}

fn build_tool_intent_completed_trace(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> ToolBatchExecutionIntentTrace {
    let tool_name = effective_result_tool_name(intent);
    let detail = summarize_completed_tool_trace_detail(tool_name.as_str(), outcome);

    ToolBatchExecutionIntentTrace {
        tool_call_id: intent.tool_call_id.clone(),
        tool_name,
        status: ToolBatchExecutionIntentStatus::Completed,
        detail,
    }
}

fn summarize_completed_tool_trace_detail(
    tool_name: &str,
    outcome: &ToolCoreOutcome,
) -> Option<String> {
    let normalized_status = outcome.status.trim();
    if !normalized_status.is_empty() && normalized_status != "ok" {
        return Some(normalized_status.to_owned());
    }

    match tool_name {
        "tool.search" => summarize_tool_search_completed_trace_detail(&outcome.payload),
        _ => None,
    }
}

fn summarize_tool_search_completed_trace_detail(payload: &serde_json::Value) -> Option<String> {
    let returned = payload.get("returned")?.as_u64()?;
    let noun = if returned == 1 { "result" } else { "results" };
    Some(format!("returned {returned} {noun}"))
}

fn build_tool_intent_failure_trace(
    intent: &ToolIntent,
    turn_result: &TurnResult,
) -> Option<ToolBatchExecutionIntentTrace> {
    let tool_name = effective_result_tool_name(intent);

    match turn_result {
        TurnResult::NeedsApproval(requirement) => Some(ToolBatchExecutionIntentTrace {
            tool_call_id: intent.tool_call_id.clone(),
            tool_name,
            status: ToolBatchExecutionIntentStatus::NeedsApproval,
            detail: Some(requirement.reason.clone()),
        }),
        TurnResult::ToolDenied(failure) => Some(ToolBatchExecutionIntentTrace {
            tool_call_id: intent.tool_call_id.clone(),
            tool_name,
            status: ToolBatchExecutionIntentStatus::Denied,
            detail: Some(failure.reason.clone()),
        }),
        TurnResult::ToolError(failure) | TurnResult::ProviderError(failure) => {
            Some(ToolBatchExecutionIntentTrace {
                tool_call_id: intent.tool_call_id.clone(),
                tool_name,
                status: ToolBatchExecutionIntentStatus::Failed,
                detail: Some(failure.reason.clone()),
            })
        }
        TurnResult::FinalText(_) | TurnResult::StreamingText(_) | TurnResult::StreamingDone(_) => {
            None
        }
    }
}

fn truncate_by_chars(value: &str, limit: usize) -> (String, usize, bool) {
    let total_chars = value.chars().count();
    if total_chars <= limit {
        return (value.to_owned(), total_chars, false);
    }
    let mut truncated = String::new();
    for ch in value.chars().take(limit) {
        truncated.push(ch);
    }
    let omitted = total_chars.saturating_sub(limit);
    truncated.push_str(&format!("...(truncated {omitted} chars)"));
    (truncated, total_chars, true)
}

fn effective_visible_tool_name(
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> String {
    if descriptor.name != "tool.invoke" {
        return descriptor.name.to_owned();
    }

    crate::tools::invoked_discoverable_tool_request(&intent.args_json)
        .map(|(tool_name, _arguments)| tool_name.to_owned())
        .unwrap_or_else(|| descriptor.name.to_owned())
}

fn provider_tool_denial_should_conceal_name(
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
    tool_is_visible: bool,
) -> bool {
    if !intent.source.starts_with("provider_") {
        return false;
    }

    if !descriptor.is_provider_exposed() {
        return true;
    }

    !tool_is_visible
        && descriptor.name == "tool.invoke"
        && effective_visible_tool_name(intent, descriptor) != descriptor.name
}

fn concealed_provider_tool_denial() -> TurnFailure {
    let base_reason = "tool_not_found: requested tool is not available";
    let reason = provider_tool_denial_reason(base_reason, "provider_tool_call");
    TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", reason)
}

fn tool_search_recovery_hint() -> &'static str {
    " If you need a non-core capability, call tool.search with a short natural-language description of the task. If tool.search returns a grouped hidden surface such as `skills`, `agent`, or `channel`, do not call that surface name directly; use tool.invoke with the fresh lease and put the requested operation inside payload.arguments."
}

fn tool_invoke_recovery_failure(reason: &str) -> Option<TurnFailure> {
    let (code, message) = if reason.starts_with("invalid_tool_lease:") {
        (
            "invalid_tool_lease",
            "tool.invoke needs a fresh lease from the current tool.search result.",
        )
    } else {
        return None;
    };

    let mut recovery_reason = message.to_owned();
    recovery_reason.push_str(tool_search_recovery_hint());

    Some(TurnFailure::policy_denied_with_discovery_recovery(
        code,
        recovery_reason,
    ))
}

fn provider_tool_denial_reason(reason: &str, source: &str) -> String {
    let is_provider_source = source.starts_with("provider_");
    if !is_provider_source {
        return reason.to_owned();
    }

    let mut message = reason.to_owned();
    message.push_str(tool_search_recovery_hint());
    message
}

fn tool_intent_is_visible(
    session_context: &SessionContext,
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> bool {
    if descriptor.is_provider_exposed() {
        if descriptor.name != "tool.invoke" {
            return true;
        }
        let effective_name = effective_visible_tool_name(intent, descriptor);
        return effective_name == descriptor.name
            || session_context.tool_view.contains(effective_name.as_str());
    }

    let provider_origin = intent.source.starts_with("provider_");
    if provider_origin {
        return false;
    }

    session_context.tool_view.contains(descriptor.name)
}

async fn execute_tool_intent_via_kernel(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
    trusted_internal_context: bool,
) -> Result<ToolCoreOutcome, TurnFailure> {
    let requested_tool_name =
        crate::tools::canonical_tool_name(request.tool_name.as_str()).to_owned();
    crate::tools::execute_kernel_tool_request(kernel_ctx, request, trusted_internal_context)
        .await
        .map_err(|error| {
            if requested_tool_name == "tool.invoke"
                && let KernelError::ToolPlane(ToolPlaneError::Execution(reason)) = &error
                && let Some(recovery_failure) = tool_invoke_recovery_failure(reason)
            {
                return recovery_failure;
            }
            if let KernelError::ToolPlane(ToolPlaneError::Execution(reason)) = &error
                && let Some(stripped) = RepairableToolPreflight::parse(reason.as_str())
            {
                let human_reason = RepairableToolPreflight::render(stripped);
                return TurnFailure::retryable("tool_preflight_denied", human_reason);
            }

            let reason = render_kernel_error_reason(&error);
            match classify_kernel_error(&error) {
                KernelFailureClass::PolicyDenied => {
                    TurnFailure::policy_denied("kernel_policy_denied", reason)
                }
                KernelFailureClass::RetryableExecution => {
                    TurnFailure::retryable("tool_execution_failed", reason)
                }
                KernelFailureClass::NonRetryable => {
                    TurnFailure::non_retryable("kernel_execution_failed", reason)
                }
            }
        })
}

struct ObserverToolRuntimeEventSink {
    observer: ConversationTurnObserverHandle,
    tool_call_id: String,
}

impl ToolRuntimeEventSink for ObserverToolRuntimeEventSink {
    fn emit(&self, event: ToolRuntimeEvent) {
        let runtime_event = ConversationTurnRuntimeEvent::new(self.tool_call_id.clone(), event);

        self.observer.on_runtime(runtime_event);
    }
}

fn build_observer_tool_runtime_event_sink(
    observer: &ConversationTurnObserverHandle,
    tool_call_id: &str,
) -> Arc<dyn ToolRuntimeEventSink> {
    let observer_sink = ObserverToolRuntimeEventSink {
        observer: Arc::clone(observer),
        tool_call_id: tool_call_id.to_owned(),
    };
    Arc::new(observer_sink)
}

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
    tool_result_payload_summary_limit_chars: usize,
    parallel_tool_execution_enabled: bool,
    parallel_tool_execution_max_in_flight: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolBatchExecutionMode {
    Sequential,
    Parallel,
}

impl ToolBatchExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreparedBatchSegment {
    len: usize,
    scheduling_class: ToolSchedulingClass,
    execution_mode: ToolBatchExecutionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionSegmentTrace {
    pub segment_index: usize,
    pub scheduling_class: ToolSchedulingClass,
    pub execution_mode: ToolBatchExecutionMode,
    pub intent_count: usize,
    pub observed_peak_in_flight: Option<usize>,
    pub observed_wall_time_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolBatchExecutionIntentStatus {
    Completed,
    NeedsApproval,
    Denied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionIntentTrace {
    pub tool_call_id: String,
    pub tool_name: String,
    pub status: ToolBatchExecutionIntentStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolDecisionTraceRecord {
    pub turn_id: String,
    pub tool_call_id: String,
    pub decision: ToolDecisionTelemetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolOutcomeTraceRecord {
    pub turn_id: String,
    pub tool_call_id: String,
    pub outcome: ToolOutcomeTelemetry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionTrace {
    pub total_intents: usize,
    pub parallel_execution_enabled: bool,
    pub parallel_execution_max_in_flight: usize,
    pub observed_peak_in_flight: usize,
    pub observed_wall_time_ms: u64,
    pub segments: Vec<ToolBatchExecutionSegmentTrace>,
    pub decision_records: Vec<ToolDecisionTraceRecord>,
    pub outcome_records: Vec<ToolOutcomeTraceRecord>,
    pub intent_outcomes: Vec<ToolBatchExecutionIntentTrace>,
}

impl ToolBatchExecutionSegmentTrace {
    fn record_observation(&mut self, observed_peak_in_flight: usize, observed_wall_time_ms: u64) {
        self.observed_peak_in_flight = Some(observed_peak_in_flight);
        self.observed_wall_time_ms = Some(observed_wall_time_ms);
    }
}

impl ToolBatchExecutionTrace {
    pub(crate) fn has_execution_segments(&self) -> bool {
        !self.segments.is_empty()
    }

    fn finish_observation(&mut self, observed_wall_time_ms: u64) {
        self.observed_wall_time_ms = observed_wall_time_ms;
        self.observed_peak_in_flight = self
            .segments
            .iter()
            .filter_map(|segment| segment.observed_peak_in_flight)
            .max()
            .unwrap_or_default();
    }

    pub(crate) fn as_event_payload(&self) -> serde_json::Value {
        let parallel_safe_intents = self
            .segments
            .iter()
            .filter(|segment| segment.scheduling_class == ToolSchedulingClass::ParallelSafe)
            .map(|segment| segment.intent_count)
            .sum::<usize>();
        let serial_only_intents = self
            .segments
            .iter()
            .filter(|segment| segment.scheduling_class == ToolSchedulingClass::SerialOnly)
            .map(|segment| segment.intent_count)
            .sum::<usize>();
        let parallel_segments = self
            .segments
            .iter()
            .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Parallel)
            .count();
        let sequential_segments = self
            .segments
            .iter()
            .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Sequential)
            .count();

        json!({
            "schema_version": 2,
            "total_intents": self.total_intents,
            "parallel_execution_enabled": self.parallel_execution_enabled,
            "parallel_execution_max_in_flight": self.parallel_execution_max_in_flight,
            "observed_peak_in_flight": self.observed_peak_in_flight,
            "observed_wall_time_ms": self.observed_wall_time_ms,
            "parallel_safe_intents": parallel_safe_intents,
            "serial_only_intents": serial_only_intents,
            "parallel_segments": parallel_segments,
            "sequential_segments": sequential_segments,
            "segments": self
                .segments
                .iter()
                .map(|segment| {
                    json!({
                        "segment_index": segment.segment_index,
                        "scheduling_class": segment.scheduling_class.as_str(),
                        "execution_mode": segment.execution_mode.as_str(),
                        "intent_count": segment.intent_count,
                        "observed_peak_in_flight": segment.observed_peak_in_flight,
                        "observed_wall_time_ms": segment.observed_wall_time_ms,
                    })
                })
                .collect::<Vec<_>>(),
        })
    }
}

fn elapsed_ms_u64(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn observe_peak_in_flight(peak: &AtomicUsize, current: usize) {
    let mut observed = peak.load(Ordering::Relaxed);
    while current > observed {
        match peak.compare_exchange_weak(observed, current, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(next) => observed = next,
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedToolIntent {
    intent_sequence: usize,
    intent: ToolIntent,
    request: ToolCoreRequest,
    execution_kind: ToolExecutionKind,
    capability_action_class: crate::tools::CapabilityActionClass,
    scheduling_class: ToolSchedulingClass,
    trusted_internal_context: bool,
    decision: ToolDecisionTelemetry,
}

#[derive(Debug, Clone)]
struct PreparedToolIntentFailure {
    intent: ToolIntent,
    turn_result: TurnResult,
    decision: ToolDecisionTelemetry,
}

#[derive(Clone, Copy)]
struct ToolIntentPreparationHarness<'a, 'b, D: AppToolDispatcher + ?Sized> {
    session_context: &'a SessionContext,
    memory_config: &'a SessionStoreConfig,
    app_dispatcher: &'a D,
    binding: ConversationRuntimeBinding<'b>,
    budget_state: &'a AutonomyTurnBudgetState,
    ingress: Option<&'a ConversationIngressContext>,
}

impl<'a, 'b, D: AppToolDispatcher + ?Sized> ToolIntentPreparationHarness<'a, 'b, D> {
    fn new(
        session_context: &'a SessionContext,
        memory_config: &'a SessionStoreConfig,
        app_dispatcher: &'a D,
        binding: ConversationRuntimeBinding<'b>,
        budget_state: &'a AutonomyTurnBudgetState,
        ingress: Option<&'a ConversationIngressContext>,
    ) -> Self {
        Self {
            session_context,
            memory_config,
            app_dispatcher,
            binding,
            budget_state,
            ingress,
        }
    }

    async fn prepare(
        self,
        intent: &ToolIntent,
        intent_sequence: usize,
    ) -> Result<PreparedToolIntent, PreparedToolIntentFailure> {
        let Some(resolved_tool) = crate::tools::resolve_tool_execution(&intent.tool_name) else {
            let denied_tool_name = effective_denied_tool_name(intent);
            let raw_reason = format!("tool_not_found: {denied_tool_name}");
            let reason = provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
            let failure = if intent.source.starts_with("provider_") {
                TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", reason.clone())
            } else {
                TurnFailure::policy_denied("tool_not_found", reason.clone())
            };
            let turn_result = TurnResult::ToolDenied(failure);
            let decision =
                ToolDecisionTelemetry::deny(denied_tool_name.as_str(), reason, "tool_not_found");

            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result,
                decision,
            });
        };

        let injected = inject_internal_tool_ingress(
            resolved_tool.canonical_name,
            intent.args_json.clone(),
            self.ingress,
        );
        let normalized_payload = crate::tools::normalize_shell_payload_for_request(
            resolved_tool.canonical_name,
            injected.payload,
        );
        let injected_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&normalized_payload);
        let augmented_payload = augment_tool_payload_for_kernel(
            resolved_tool.canonical_name,
            normalized_payload.clone(),
            self.session_context,
            self.memory_config,
        );
        let augmented_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&augmented_payload.payload);
        let request = ToolCoreRequest {
            tool_name: resolved_tool.canonical_name.to_owned(),
            payload: augmented_payload.payload,
        };
        let request = prepare_conversation_kernel_tool_request(request, self.binding, intent);
        let normalized_intent = ToolIntent {
            tool_name: resolved_tool.canonical_name.to_owned(),
            args_json: normalized_payload,
            source: intent.source.clone(),
            session_id: intent.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
        };
        let effective_tool_metadata =
            resolve_effective_tool_metadata(resolved_tool, request, normalized_intent, intent);
        let effective_tool_metadata = match effective_tool_metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                let effective_target = error.effective_target;
                let effective_tool_name = effective_target.tool_name;
                let effective_intent = effective_target.intent;
                let reason = format!("tool_descriptor_missing: {}", effective_tool_name);
                let turn_result =
                    TurnResult::non_retryable_tool_error("tool_descriptor_missing", reason.clone());
                let decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_descriptor_missing",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
        };
        let effective_execution_kind = effective_tool_metadata.execution_kind;
        let effective_request = effective_tool_metadata.request;
        let effective_intent = effective_tool_metadata.intent;
        let effective_tool_name = effective_tool_metadata.tool_name;
        let descriptor = effective_tool_metadata.descriptor;
        let capability_action_class = effective_tool_metadata.capability_action_class;
        let scheduling_class = effective_tool_metadata.scheduling_class;

        let decision = match self
            .app_dispatcher
            .preflight_tool_intent_with_binding(
                self.session_context,
                &effective_intent,
                &descriptor,
                self.binding,
                self.budget_state,
            )
            .await
        {
            Ok(ToolPreflightOutcome::Allow(decision)) => decision,
            Ok(ToolPreflightOutcome::NeedsApproval {
                requirement,
                decision,
            }) => {
                let turn_result = TurnResult::NeedsApproval(requirement);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
            Ok(ToolPreflightOutcome::Denied { failure, decision }) => {
                let turn_result = TurnResult::ToolDenied(failure);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
            Err(reason) if reason.starts_with("app_tool_denied:") => {
                let human_reason = render_app_tool_denied_reason(reason.as_str());
                let turn_result =
                    TurnResult::policy_denied("app_tool_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "app_tool_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) => {
                let turn_result =
                    TurnResult::non_retryable_tool_error("tool_preflight_failed", reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_preflight_failed",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
        };

        let requires_kernel_binding = match effective_execution_kind {
            ToolExecutionKind::Core => true,
            ToolExecutionKind::App => descriptor.requires_kernel_binding(),
        };
        let has_kernel_context = self.binding.kernel_context().is_some();

        if requires_kernel_binding && !has_kernel_context {
            let turn_result = TurnResult::policy_denied("no_kernel_context", "no_kernel_context");
            let denial_decision = ToolDecisionTelemetry::deny(
                effective_tool_name.as_str(),
                "no_kernel_context",
                "no_kernel_context",
            );

            return Err(PreparedToolIntentFailure {
                intent: effective_intent,
                turn_result,
                decision: denial_decision,
            });
        }

        let preflight = self
            .app_dispatcher
            .preflight_tool_execution_with_binding(
                self.session_context,
                &effective_intent,
                effective_request,
                &descriptor,
                self.binding,
            )
            .await;

        let (effective_request, trusted_preflight_context) = match preflight {
            Ok(ToolExecutionPreflight::Ready {
                request,
                trusted_internal_context,
            }) => (request, trusted_internal_context),
            Ok(ToolExecutionPreflight::NeedsApproval(requirement)) => {
                let turn_result = TurnResult::NeedsApproval(requirement.clone());
                let approval_decision =
                    approval_required_tool_decision(effective_tool_name.as_str(), &requirement);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: approval_decision,
                });
            }
            Err(reason) if reason.starts_with("app_tool_denied:") => {
                let human_reason = render_app_tool_denied_reason(reason.as_str());
                let turn_result =
                    TurnResult::policy_denied("app_tool_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "app_tool_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) if RepairableToolPreflight::parse(reason.as_str()).is_some() => {
                let stripped =
                    RepairableToolPreflight::parse(reason.as_str()).unwrap_or(reason.as_str());
                let human_reason = RepairableToolPreflight::render(stripped);
                let turn_result =
                    TurnResult::retryable_tool_error("tool_preflight_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "tool_preflight_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) if reason.starts_with("tool_preflight_denied:") => {
                let turn_result =
                    TurnResult::policy_denied("tool_preflight_denied", reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_preflight_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) => {
                let turn_result = TurnResult::non_retryable_tool_error(
                    "app_tool_preflight_failed",
                    reason.clone(),
                );
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "app_tool_preflight_failed",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
        };

        let injected_trusted_internal_context = injected.trusted_internal_context
            || augmented_payload.trusted_internal_context
            || (!injected_payload_uses_reserved_internal_context
                && augmented_payload_uses_reserved_internal_context);
        let trusted_internal_context =
            injected_trusted_internal_context || trusted_preflight_context;

        Ok(PreparedToolIntent {
            intent_sequence,
            intent: effective_intent,
            request: effective_request,
            execution_kind: effective_execution_kind,
            capability_action_class,
            scheduling_class,
            trusted_internal_context,
            decision,
        })
    }
}

#[derive(Clone, Copy)]
struct ToolBatchHarness<'a> {
    engine: &'a TurnEngine,
}

impl<'a> ToolBatchHarness<'a> {
    fn new(engine: &'a TurnEngine) -> Self {
        Self { engine }
    }

    fn trace_empty_batch(self, total_intents: usize) -> ToolBatchExecutionTrace {
        ToolBatchExecutionTrace {
            total_intents,
            parallel_execution_enabled: self.engine.parallel_tool_execution_enabled,
            parallel_execution_max_in_flight: self.engine.parallel_tool_execution_max_in_flight,
            observed_peak_in_flight: 0,
            observed_wall_time_ms: 0,
            segments: Vec::new(),
            decision_records: Vec::new(),
            outcome_records: Vec::new(),
            intent_outcomes: Vec::new(),
        }
    }

    fn populate_trace_segments(
        self,
        trace: &mut ToolBatchExecutionTrace,
        batch_segments: &[PreparedBatchSegment],
    ) {
        trace.parallel_execution_enabled = self.engine.parallel_tool_execution_enabled;
        trace.parallel_execution_max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        trace.segments = batch_segments
            .iter()
            .enumerate()
            .map(|(segment_index, segment)| ToolBatchExecutionSegmentTrace {
                segment_index,
                scheduling_class: segment.scheduling_class,
                execution_mode: segment.execution_mode,
                intent_count: segment.len,
                observed_peak_in_flight: None,
                observed_wall_time_ms: None,
            })
            .collect();
    }

    fn prepared_batch_segments(self, prepared: &[PreparedToolIntent]) -> Vec<PreparedBatchSegment> {
        let mut segments = Vec::new();
        let mut remaining = prepared;

        while let Some((first, _)) = remaining.split_first() {
            let scheduling_class = first.scheduling_class;
            let len = remaining
                .iter()
                .take_while(|prepared_intent| prepared_intent.scheduling_class == scheduling_class)
                .count();
            let execution_mode = self.segment_execution_mode(scheduling_class, len);

            segments.push(PreparedBatchSegment {
                len,
                scheduling_class,
                execution_mode,
            });

            let (_, rest) = remaining.split_at(len);
            remaining = rest;
        }

        segments
    }

    fn segment_execution_mode(
        self,
        scheduling_class: ToolSchedulingClass,
        segment_len: usize,
    ) -> ToolBatchExecutionMode {
        let parallel_enabled = self.engine.parallel_tool_execution_enabled;
        let max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        let is_parallel_safe = scheduling_class == ToolSchedulingClass::ParallelSafe;
        let has_multiple_intents = segment_len > 1;

        if parallel_enabled && max_in_flight > 1 && is_parallel_safe && has_multiple_intents {
            return ToolBatchExecutionMode::Parallel;
        }

        ToolBatchExecutionMode::Sequential
    }

    async fn execute_prepared_batch<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        batch_segments: &[PreparedBatchSegment],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        trace: &mut ToolBatchExecutionTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let result = async {
            let mut outputs = Vec::with_capacity(prepared.len());
            let mut remaining = prepared;

            debug_assert_eq!(trace.segments.len(), batch_segments.len());

            for (segment, trace_segment) in batch_segments
                .iter()
                .copied()
                .zip(trace.segments.iter_mut())
            {
                let (prepared_segment, rest) = remaining.split_at(segment.len);
                let mut segment_outputs = match segment.execution_mode {
                    ToolBatchExecutionMode::Parallel => {
                        self.execute_prepared_batch_in_parallel(
                            prepared_segment,
                            session_context,
                            app_dispatcher,
                            binding,
                            &mut trace.intent_outcomes,
                            &mut trace.outcome_records,
                            trace_segment,
                            observer,
                        )
                        .await?
                    }
                    ToolBatchExecutionMode::Sequential => {
                        self.execute_prepared_batch_sequential(
                            prepared_segment,
                            session_context,
                            app_dispatcher,
                            binding,
                            &mut trace.intent_outcomes,
                            &mut trace.outcome_records,
                            trace_segment,
                            observer,
                        )
                        .await?
                    }
                };

                outputs.append(&mut segment_outputs);
                remaining = rest;
            }

            Ok(outputs)
        }
        .await;

        trace.finish_observation(elapsed_ms_u64(started_at));

        result
    }

    async fn execute_prepared_batch_sequential<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        intent_outcomes: &mut Vec<ToolBatchExecutionIntentTrace>,
        outcome_records: &mut Vec<ToolOutcomeTraceRecord>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let result = async {
            let mut outputs = Vec::with_capacity(prepared.len());

            for prepared_intent in prepared {
                let outcome = match self
                    .engine
                    .execute_prepared_tool_intent(
                        prepared_intent,
                        session_context,
                        app_dispatcher,
                        binding,
                        observer,
                    )
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(turn_result) => {
                        let outcome_record = build_failure_tool_outcome_trace_record(
                            &prepared_intent.intent,
                            &turn_result,
                        );

                        if let Some(outcome_record) = outcome_record {
                            outcome_records.push(outcome_record);
                        }

                        let intent_outcome =
                            build_tool_intent_failure_trace(&prepared_intent.intent, &turn_result);

                        if let Some(intent_outcome) = intent_outcome {
                            intent_outcomes.push(intent_outcome);
                        }

                        return Err(turn_result);
                    }
                };

                app_dispatcher
                    .after_tool_execution(
                        session_context,
                        &prepared_intent.intent,
                        prepared_intent.intent_sequence,
                        &prepared_intent.request,
                        &outcome,
                        binding,
                    )
                    .await;

                let outcome_record =
                    build_success_tool_outcome_trace_record(&prepared_intent.intent, &outcome);
                outcome_records.push(outcome_record);

                let intent_outcome =
                    build_tool_intent_completed_trace(&prepared_intent.intent, &outcome);
                intent_outcomes.push(intent_outcome);

                let payload_summary_limit_chars =
                    self.engine.tool_result_payload_summary_limit_chars;
                let output = format_tool_result_line_with_limit(
                    &prepared_intent.intent,
                    &outcome,
                    payload_summary_limit_chars,
                );
                outputs.push(output);
            }

            Ok(outputs)
        }
        .await;

        let observed_peak_in_flight = if prepared.is_empty() { 0 } else { 1 };
        let observed_wall_time_ms = elapsed_ms_u64(started_at);
        trace_segment.record_observation(observed_peak_in_flight, observed_wall_time_ms);

        result
    }

    async fn execute_prepared_batch_in_parallel<D: AppToolDispatcher + ?Sized>(
        self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        intent_outcomes: &mut Vec<ToolBatchExecutionIntentTrace>,
        outcome_records: &mut Vec<ToolOutcomeTraceRecord>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let payload_summary_limit_chars = self.engine.tool_result_payload_summary_limit_chars;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let observed_peak = Arc::new(AtomicUsize::new(0));
        let mut indexed_intent_outcomes = Vec::with_capacity(prepared.len());
        let mut indexed_outcome_records = Vec::with_capacity(prepared.len());
        let mut results = Vec::with_capacity(prepared.len());
        let max_in_flight = self.engine.parallel_tool_execution_max_in_flight;
        let mut executions = stream::iter(prepared.iter().cloned().enumerate().map(
            |(index, prepared_intent)| {
                let in_flight = Arc::clone(&in_flight);
                let observed_peak = Arc::clone(&observed_peak);

                async move {
                    let current_in_flight = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                    observe_peak_in_flight(observed_peak.as_ref(), current_in_flight);

                    let result = match self
                        .engine
                        .execute_prepared_tool_intent(
                            &prepared_intent,
                            session_context,
                            app_dispatcher,
                            binding,
                            observer,
                        )
                        .await
                    {
                        Ok(outcome) => {
                            app_dispatcher
                                .after_tool_execution(
                                    session_context,
                                    &prepared_intent.intent,
                                    prepared_intent.intent_sequence,
                                    &prepared_intent.request,
                                    &outcome,
                                    binding,
                                )
                                .await;

                            let output = format_tool_result_line_with_limit(
                                &prepared_intent.intent,
                                &outcome,
                                payload_summary_limit_chars,
                            );
                            let outcome_record = build_success_tool_outcome_trace_record(
                                &prepared_intent.intent,
                                &outcome,
                            );
                            let intent_outcome = build_tool_intent_completed_trace(
                                &prepared_intent.intent,
                                &outcome,
                            );

                            Ok((output, intent_outcome, outcome_record))
                        }
                        Err(turn_result) => {
                            let intent_outcome = build_tool_intent_failure_trace(
                                &prepared_intent.intent,
                                &turn_result,
                            );
                            let outcome_record = build_failure_tool_outcome_trace_record(
                                &prepared_intent.intent,
                                &turn_result,
                            );

                            Err((turn_result, intent_outcome, outcome_record))
                        }
                    };

                    in_flight.fetch_sub(1, Ordering::Relaxed);

                    (index, result)
                }
            },
        ))
        .buffer_unordered(max_in_flight);

        let mut batch_failure = None;
        while let Some((index, result)) = executions.next().await {
            match result {
                Ok((output, intent_outcome, outcome_record)) => {
                    indexed_intent_outcomes.push((index, intent_outcome));
                    indexed_outcome_records.push((index, outcome_record));
                    results.push((index, output));
                }
                Err((turn_result, intent_outcome, outcome_record)) => {
                    if let Some(intent_outcome) = intent_outcome {
                        indexed_intent_outcomes.push((index, intent_outcome));
                    }

                    if let Some(outcome_record) = outcome_record {
                        indexed_outcome_records.push((index, outcome_record));
                    }

                    batch_failure = Some(turn_result);
                    break;
                }
            }
        }

        let observed_peak_in_flight = observed_peak.load(Ordering::Relaxed);
        let observed_wall_time_ms = elapsed_ms_u64(started_at);
        trace_segment.record_observation(observed_peak_in_flight, observed_wall_time_ms);
        results.sort_by_key(|(index, _)| *index);
        indexed_intent_outcomes.sort_by_key(|(index, _)| *index);
        indexed_outcome_records.sort_by_key(|(index, _)| *index);
        intent_outcomes.extend(
            indexed_intent_outcomes
                .into_iter()
                .map(|(_, intent_outcome)| intent_outcome),
        );
        outcome_records.extend(
            indexed_outcome_records
                .into_iter()
                .map(|(_, outcome_record)| outcome_record),
        );

        if let Some(turn_result) = batch_failure {
            return Err(turn_result);
        }

        Ok(results.into_iter().map(|(_, output)| output).collect())
    }
}

#[derive(Debug, Clone)]
struct EffectiveToolTarget {
    execution_kind: ToolExecutionKind,
    request: ToolCoreRequest,
    intent: ToolIntent,
    tool_name: String,
}

#[derive(Debug, Clone)]
struct EffectiveToolMetadata {
    execution_kind: ToolExecutionKind,
    request: ToolCoreRequest,
    intent: ToolIntent,
    tool_name: String,
    descriptor: ToolDescriptor,
    capability_action_class: crate::tools::CapabilityActionClass,
    scheduling_class: ToolSchedulingClass,
}

#[derive(Debug, Clone)]
struct EffectiveToolMetadataError {
    effective_target: EffectiveToolTarget,
}

/// Resolve the runtime execution target for one provider-emitted tool intent.
///
/// Most tools execute as-is. `tool.invoke` is special: it may need to borrow
/// metadata from the discovered inner tool and, for app tools or shell exec,
/// rebind the executable request itself so downstream governance sees the real
/// operation rather than only the wrapper.
fn resolve_effective_tool_target(
    resolved_tool: ResolvedToolExecution,
    request: ToolCoreRequest,
    normalized_intent: ToolIntent,
    original_intent: &ToolIntent,
) -> EffectiveToolTarget {
    if resolved_tool.canonical_name != "tool.invoke" {
        let execution_kind = resolved_tool.execution_kind;
        let tool_name = resolved_tool.canonical_name.to_owned();

        return EffectiveToolTarget {
            execution_kind,
            request,
            intent: normalized_intent,
            tool_name,
        };
    }

    let invoke_resolution = crate::tools::resolve_tool_invoke_request(&request);

    let Ok((inner_resolved, inner_request)) = invoke_resolution else {
        let execution_kind = resolved_tool.execution_kind;
        let tool_name = resolved_tool.canonical_name.to_owned();

        return EffectiveToolTarget {
            execution_kind,
            request,
            intent: normalized_intent,
            tool_name,
        };
    };

    let inner_intent = ToolIntent {
        tool_name: inner_resolved.canonical_name.to_owned(),
        args_json: inner_request.payload.clone(),
        source: original_intent.source.clone(),
        session_id: original_intent.session_id.clone(),
        turn_id: original_intent.turn_id.clone(),
        tool_call_id: original_intent.tool_call_id.clone(),
    };

    let rebind_for_app_tool = inner_resolved.execution_kind == ToolExecutionKind::App;
    let inner_tool_name = inner_resolved.canonical_name;
    let rebind_for_shell_exec = inner_tool_name == crate::tools::SHELL_EXEC_TOOL_NAME;
    let should_rebind_request = rebind_for_app_tool || rebind_for_shell_exec;

    if should_rebind_request {
        let execution_kind = inner_resolved.execution_kind;
        let tool_name = inner_resolved.canonical_name.to_owned();

        return EffectiveToolTarget {
            execution_kind,
            request: inner_request,
            intent: inner_intent,
            tool_name,
        };
    }

    let execution_kind = resolved_tool.execution_kind;
    let tool_name = inner_resolved.canonical_name.to_owned();

    EffectiveToolTarget {
        execution_kind,
        request,
        intent: inner_intent,
        tool_name,
    }
}

fn resolve_effective_tool_descriptor(effective_tool_name: &str) -> Option<ToolDescriptor> {
    let catalog = crate::tools::tool_catalog();
    let direct_descriptor = catalog.resolve(effective_tool_name);
    direct_descriptor.copied()
}

fn resolve_effective_tool_metadata(
    resolved_tool: ResolvedToolExecution,
    request: ToolCoreRequest,
    normalized_intent: ToolIntent,
    original_intent: &ToolIntent,
) -> Result<EffectiveToolMetadata, Box<EffectiveToolMetadataError>> {
    let effective_target =
        resolve_effective_tool_target(resolved_tool, request, normalized_intent, original_intent);
    let descriptor = resolve_effective_tool_descriptor(effective_target.tool_name.as_str());
    let Some(descriptor) = descriptor else {
        let error = EffectiveToolMetadataError { effective_target };

        return Err(Box::new(error));
    };

    let capability_action_class = descriptor.capability_action_class();
    let scheduling_class = descriptor.scheduling_class();

    Ok(EffectiveToolMetadata {
        execution_kind: effective_target.execution_kind,
        request: effective_target.request,
        intent: effective_target.intent,
        tool_name: effective_target.tool_name,
        descriptor,
        capability_action_class,
        scheduling_class,
    })
}

fn prepare_conversation_kernel_tool_request(
    request: ToolCoreRequest,
    binding: ConversationRuntimeBinding<'_>,
    _intent: &ToolIntent,
) -> ToolCoreRequest {
    let Some(kernel_ctx) = binding.kernel_context() else {
        return request;
    };

    let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
    if canonical_tool_name != "tool.search" {
        return request;
    }

    crate::tools::prepare_kernel_tool_request(
        request,
        &kernel_ctx.token.allowed_capabilities,
        None,
        None,
        None,
    )
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self::with_parallel_tool_execution(
            max_tool_steps,
            TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
            false,
            1,
        )
    }

    pub fn with_tool_result_payload_summary_limit(
        max_tool_steps: usize,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self::with_parallel_tool_execution(
            max_tool_steps,
            tool_result_payload_summary_limit_chars,
            false,
            1,
        )
    }

    pub fn with_parallel_tool_execution(
        max_tool_steps: usize,
        tool_result_payload_summary_limit_chars: usize,
        parallel_tool_execution_enabled: bool,
        parallel_tool_execution_max_in_flight: usize,
    ) -> Self {
        Self {
            max_tool_steps,
            tool_result_payload_summary_limit_chars: tool_result_payload_summary_limit_chars.clamp(
                MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
                MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
            ),
            parallel_tool_execution_enabled,
            parallel_tool_execution_max_in_flight: parallel_tool_execution_max_in_flight.max(1),
        }
    }

    fn tool_batch_harness(&self) -> ToolBatchHarness<'_> {
        ToolBatchHarness::new(self)
    }

    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        self.evaluate_turn_in_view(turn, &runtime_tool_view())
    }

    pub fn evaluate_turn_in_view(&self, turn: &ProviderTurn, tool_view: &ToolView) -> TurnResult {
        self.evaluate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn evaluate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> TurnResult {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => TurnResult::FinalText(text),
            Err(failure) => TurnResult::ToolDenied(failure),
            Ok(TurnValidation::ToolExecutionRequired) => {
                TurnResult::policy_denied("kernel_context_required", "kernel_context_required")
            }
        }
    }

    /// Validate a provider turn and describe whether tool execution is needed.
    ///
    /// This phase is pure: it validates the turn shape and tool budget, but it does
    /// not make runtime binding decisions about whether a kernel is available.
    pub fn validate_turn(&self, turn: &ProviderTurn) -> Result<TurnValidation, TurnFailure> {
        self.validate_turn_in_view(turn, &runtime_tool_view())
    }

    pub fn validate_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
    ) -> Result<TurnValidation, TurnFailure> {
        self.validate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn validate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> Result<TurnValidation, TurnFailure> {
        if turn.tool_intents.is_empty() {
            return Ok(TurnValidation::FinalText(turn.assistant_text.clone()));
        }

        if turn.tool_intents.len() > self.max_tool_steps {
            return Err(TurnFailure::policy_denied(
                "max_tool_steps_exceeded",
                "max_tool_steps_exceeded",
            ));
        }

        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(resolved_tool) = crate::tools::resolve_tool_execution(&intent.tool_name)
            else {
                let raw_reason = format!("tool_not_found: {}", intent.tool_name);
                let reason =
                    provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
                let failure = if intent.source.starts_with("provider_") {
                    TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", reason)
                } else {
                    TurnFailure::policy_denied("tool_not_found", reason)
                };
                return Err(failure);
            };
            if let Some(descriptor) = catalog.resolve(&intent.tool_name) {
                let tool_is_visible = tool_intent_is_visible(session_context, intent, descriptor);
                if !tool_is_visible {
                    if provider_tool_denial_should_conceal_name(intent, descriptor, false) {
                        return Err(concealed_provider_tool_denial());
                    }
                    let reason = format!(
                        "tool_not_visible: {}",
                        effective_visible_tool_name(intent, descriptor)
                    );
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }
                if provider_tool_denial_should_conceal_name(intent, descriptor, true) {
                    return Err(concealed_provider_tool_denial());
                }
                // For tool.invoke, the inner tool_id is validated by lease during
                // execution.  We do not check inner visibility here because discoverable
                // tools are intentionally hidden from the tool_view and accessed only
                // through a valid lease obtained from tool.search.
                // For all other provider-sourced intents, verify they are provider-exposed
                // (this gate catches non-bridge paths where a discoverable tool name
                // arrives without being rewritten to tool.invoke).
                if tool_intent_skips_provider_exposed_gate(intent, descriptor) {
                    // Lease validation happens in resolve_tool_invoke_request during execution.
                    // Internal approval-control turns also bypass provider exposure checks for
                    // the approval tools they synthesize.
                } else if !crate::tools::is_provider_exposed_tool_name(&intent.tool_name) {
                    let reason = format!("tool_not_provider_exposed: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied(
                        "tool_not_provider_exposed",
                        reason,
                    ));
                }
            } else {
                if intent.source.starts_with("provider_") {
                    return Err(concealed_provider_tool_denial());
                }
                if !session_context
                    .tool_view
                    .contains(resolved_tool.canonical_name)
                {
                    let reason = format!("tool_not_visible: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }
            }
        }

        Ok(TurnValidation::ToolExecutionRequired)
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. Policy/capability check via kernel → `ToolDenied`
    /// 5. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: &KernelContext,
    ) -> TurnResult {
        self.execute_turn_in_view(
            turn,
            &runtime_tool_view(),
            ConversationRuntimeBinding::kernel(kernel_ctx),
        )
        .await
    }

    pub async fn execute_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, tool_view.clone()),
            &DefaultAppToolDispatcher::runtime(),
            binding,
            None,
        )
        .await
    }

    pub async fn execute_turn_with_ingress(
        &self,
        turn: &ProviderTurn,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, runtime_tool_view()),
            &DefaultAppToolDispatcher::runtime(),
            binding,
            ingress,
        )
        .await
    }

    pub async fn execute_turn_in_context<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> TurnResult {
        self.execute_turn_in_context_with_trace(
            turn,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            None,
        )
        .await
        .0
    }

    pub(crate) async fn execute_turn_in_context_with_trace<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> (TurnResult, Option<ToolBatchExecutionTrace>) {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => return (TurnResult::FinalText(text), None),
            Err(failure) => return (TurnResult::ToolDenied(failure), None),
            Ok(TurnValidation::ToolExecutionRequired) => {}
        }

        let tool_batch_harness = self.tool_batch_harness();
        let mut trace = tool_batch_harness.trace_empty_batch(turn.tool_intents.len());
        let mut prepared = Vec::new();
        let mut autonomy_budget_state = AutonomyTurnBudgetState::default();
        for (intent_sequence, intent) in turn.tool_intents.iter().enumerate() {
            match self
                .prepare_tool_intent(
                    intent,
                    intent_sequence,
                    session_context,
                    app_dispatcher,
                    binding,
                    &autonomy_budget_state,
                    ingress,
                )
                .await
            {
                Ok(prepared_intent) => {
                    let decision_record = build_tool_decision_trace_record(
                        &prepared_intent.intent,
                        prepared_intent.decision.clone(),
                    );
                    trace.decision_records.push(decision_record);
                    autonomy_budget_state.record_action(prepared_intent.capability_action_class);
                    prepared.push(prepared_intent);
                }
                Err(failure) => {
                    let decision_record =
                        build_tool_decision_trace_record(&failure.intent, failure.decision);
                    trace.decision_records.push(decision_record);
                    let intent_outcome =
                        build_tool_intent_failure_trace(&failure.intent, &failure.turn_result);
                    if let Some(intent_outcome) = intent_outcome {
                        trace.intent_outcomes.push(intent_outcome);
                    }
                    return (failure.turn_result, Some(trace));
                }
            }
        }
        let batch_segments = tool_batch_harness.prepared_batch_segments(&prepared);
        tool_batch_harness.populate_trace_segments(&mut trace, &batch_segments);

        let outputs = match tool_batch_harness
            .execute_prepared_batch(
                &prepared,
                &batch_segments,
                session_context,
                app_dispatcher,
                binding,
                &mut trace,
                observer,
            )
            .await
        {
            Ok(outputs) => outputs,
            Err(result) => return (result, Some(trace)),
        };

        (TurnResult::FinalText(outputs.join("\n")), Some(trace))
    }

    async fn prepare_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        intent: &ToolIntent,
        intent_sequence: usize,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &AutonomyTurnBudgetState,
        ingress: Option<&ConversationIngressContext>,
    ) -> Result<PreparedToolIntent, PreparedToolIntentFailure> {
        let memory_config = app_dispatcher
            .memory_config()
            .unwrap_or(store::current_session_store_config());
        let preparation_harness = ToolIntentPreparationHarness::new(
            session_context,
            memory_config,
            app_dispatcher,
            binding,
            budget_state,
            ingress,
        );
        preparation_harness.prepare(intent, intent_sequence).await
    }

    async fn execute_prepared_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared_intent: &PreparedToolIntent,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> Result<ToolCoreOutcome, TurnResult> {
        match prepared_intent.execution_kind {
            ToolExecutionKind::Core => {
                let Some(kernel_ctx) = binding.kernel_context() else {
                    return Err(TurnResult::policy_denied(
                        "no_kernel_context",
                        "no_kernel_context",
                    ));
                };
                let execution = execute_tool_intent_via_kernel(
                    prepared_intent.request.clone(),
                    kernel_ctx,
                    prepared_intent.trusted_internal_context,
                );
                let outcome = match observer {
                    Some(observer) => {
                        let sink = build_observer_tool_runtime_event_sink(
                            observer,
                            prepared_intent.intent.tool_call_id.as_str(),
                        );

                        with_tool_runtime_event_sink(sink, execution).await
                    }
                    None => execution.await,
                };

                outcome.map_err(turn_result_from_tool_execution_failure)
            }
            ToolExecutionKind::App => match app_dispatcher
                .execute_app_tool(session_context, prepared_intent.request.clone(), binding)
                .await
            {
                Ok(outcome) => Ok(outcome),
                Err(reason) if reason.starts_with("tool_not_visible:") => {
                    Err(TurnResult::policy_denied("tool_not_visible", reason))
                }
                Err(reason)
                    if reason.starts_with("tool_not_found:")
                        || reason.starts_with("app_tool_not_found:") =>
                {
                    let policy_reason = provider_tool_denial_reason(
                        reason.as_str(),
                        prepared_intent.intent.source.as_str(),
                    );
                    let failure = if prepared_intent.intent.source.starts_with("provider_") {
                        TurnFailure::policy_denied_with_discovery_recovery(
                            "tool_not_found",
                            policy_reason,
                        )
                    } else {
                        TurnFailure::policy_denied("tool_not_found", policy_reason)
                    };
                    Err(TurnResult::ToolDenied(failure))
                }
                Err(reason) if reason.starts_with("app_tool_disabled:") => {
                    Err(TurnResult::policy_denied("app_tool_disabled", reason))
                }
                Err(reason) if reason.starts_with("app_tool_denied:") => {
                    let human_reason = render_app_tool_denied_reason(reason.as_str());
                    Err(TurnResult::policy_denied("app_tool_denied", human_reason))
                }
                Err(reason) => Err(TurnResult::non_retryable_tool_error(
                    "app_tool_execution_failed",
                    reason,
                )),
            },
        }
    }
}

fn session_context_from_turn(turn: &ProviderTurn, tool_view: ToolView) -> SessionContext {
    let session_id = turn
        .tool_intents
        .first()
        .map(|intent| intent.session_id.as_str())
        .unwrap_or("default");
    SessionContext::root_with_tool_view(session_id, tool_view)
}

#[cfg(test)]
mod tests {
    use crate::context::bootstrap_test_kernel_context;
    use crate::test_support::unique_temp_dir;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::config::{AutonomyProfile, GovernedToolApprovalMode, ToolConfig};
    use crate::session::repository::{
        ApprovalRequestStatus, NewApprovalGrantRecord, NewSessionEvent, NewSessionRecord,
        SessionKind, SessionRepository, SessionState,
    };

    fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
        let base = std::env::temp_dir().join(format!(
            "loong-turn-engine-approval-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        SessionStoreConfig {
            sqlite_path: Some(db_path),
            runtime_config: None,
        }
    }

    fn test_kernel_context(agent_id: &str) -> KernelContext {
        crate::context::bootstrap_test_kernel_context(agent_id, 60)
            .expect("bootstrap test kernel context")
    }

    fn kernel_context(agent_id: &str) -> KernelContext {
        test_kernel_context(agent_id)
    }

    #[test]
    fn validate_turn_in_context_conceals_provider_hidden_tool_invoke_alias_denial() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "tool_invoke".to_owned(),
                args_json: json!({
                    "tool_id": "shell.exec",
                    "arguments": {"command": "echo hello"},
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "session-provider-hidden-tool-invoke".to_owned(),
                turn_id: "turn-provider-hidden-tool-invoke".to_owned(),
                tool_call_id: "call-provider-hidden-tool-invoke".to_owned(),
            }],
            raw_meta: Value::Null,
        };
        let session_context = SessionContext::root_with_tool_view(
            "session-provider-hidden-tool-invoke",
            crate::tools::ToolView::from_tool_names(std::iter::empty::<&str>()),
        );

        let failure = TurnEngine::new(4)
            .validate_turn_in_context(&turn, &session_context)
            .expect_err("provider hidden tool.invoke alias should be concealed");

        assert_eq!(failure.code, "tool_not_found");
        assert!(
            failure
                .reason
                .contains("tool_not_found: requested tool is not available")
        );
        assert!(
            failure.reason.contains("tool.search"),
            "concealed denial should advertise discovery recovery: {}",
            failure.reason
        );
        assert!(failure.supports_discovery_recovery);
        assert!(
            failure.reason.contains("tool.invoke"),
            "concealed denial should advertise tool.invoke recovery: {}",
            failure.reason
        );
        assert!(
            failure.reason.contains("lease"),
            "concealed denial should mention the lease requirement: {}",
            failure.reason
        );
    }

    #[test]
    fn validate_turn_in_context_conceals_direct_hidden_skills_surface_and_advertises_lease_flow() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "skills".to_owned(),
                args_json: json!({
                    "operation": "list"
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "session-provider-direct-skills".to_owned(),
                turn_id: "turn-provider-direct-skills".to_owned(),
                tool_call_id: "call-provider-direct-skills".to_owned(),
            }],
            raw_meta: Value::Null,
        };
        let session_context = SessionContext::root_with_tool_view(
            "session-provider-direct-skills",
            crate::tools::ToolView::from_tool_names(std::iter::empty::<&str>()),
        );

        let failure = TurnEngine::new(4)
            .validate_turn_in_context(&turn, &session_context)
            .expect_err("provider direct hidden skills surface should be concealed");

        assert_eq!(failure.code, "tool_not_found");
        assert!(
            failure
                .reason
                .contains("tool_not_found: requested tool is not available")
        );
        assert!(
            failure.reason.contains("tool.search"),
            "concealed denial should advertise discovery recovery: {}",
            failure.reason
        );
        assert!(
            failure.reason.contains("tool.invoke"),
            "concealed denial should explain the grouped-surface invoke flow: {}",
            failure.reason
        );
        assert!(
            failure.reason.contains("skills"),
            "concealed denial should mention grouped hidden surfaces: {}",
            failure.reason
        );
        assert!(failure.supports_discovery_recovery);
    }

    #[test]
    fn validate_turn_in_context_allows_internal_approval_control_resolve_tool() {
        let turn = ProviderTurn {
            assistant_text: String::new(),
            tool_intents: vec![ToolIntent {
                tool_name: "approval_request_resolve".to_owned(),
                args_json: json!({
                    "approval_request_id": "apr-allow-1",
                    "decision": "approve_once"
                }),
                source: "approval_control".to_owned(),
                session_id: "session-approval-control".to_owned(),
                turn_id: "turn-approval-control".to_owned(),
                tool_call_id: "call-approval-control".to_owned(),
            }],
            raw_meta: Value::Null,
        };
        let tool_view = crate::tools::ToolView::from_tool_names([
            "approval_request_resolve",
            "approval_request_status",
            "approval_requests_list",
        ]);
        let session_context =
            SessionContext::root_with_tool_view("session-approval-control", tool_view);

        let validation = TurnEngine::new(4)
            .validate_turn_in_context(&turn, &session_context)
            .expect("approval-control resolve should stay executable");

        assert_eq!(validation, TurnValidation::ToolExecutionRequired);
    }

    #[test]
    fn prepare_tool_intent_uses_inner_shell_metadata_for_tool_invoke_core_requests() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"],
            }),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "session-shell-invoke-trace".to_owned(),
            turn_id: "turn-shell-invoke-trace".to_owned(),
            tool_call_id: "call-shell-invoke-trace".to_owned(),
        };
        let session_context =
            SessionContext::root_with_tool_view("session-shell-invoke-trace", runtime_tool_view());
        let engine = TurnEngine::new(4);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let prepared_intent = runtime.block_on(async {
            let autonomy_budget_state = AutonomyTurnBudgetState::default();
            engine
                .prepare_tool_intent(
                    &intent,
                    0,
                    &session_context,
                    &DefaultAppToolDispatcher::runtime(),
                    ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                    &autonomy_budget_state,
                    None,
                )
                .await
                .expect("tool.invoke shell request should prepare successfully")
        });

        assert_eq!(prepared_intent.request.tool_name, "shell.exec");
        assert_eq!(prepared_intent.intent.tool_name, "shell.exec");
        assert_eq!(
            prepared_intent.intent.args_json,
            json!({
                "command": "echo",
                "args": ["hello"],
            })
        );
    }

    #[test]
    fn prepare_tool_intent_injects_capability_filter_prep_for_tool_search() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let intent = ToolIntent {
            tool_name: "tool.search".to_owned(),
            args_json: json!({
                "query": "read note.md",
                "limit": 3,
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "session-tool-search-prep".to_owned(),
            turn_id: "turn-tool-search-prep".to_owned(),
            tool_call_id: "call-tool-search-prep".to_owned(),
        };
        let session_context =
            SessionContext::root_with_tool_view("session-tool-search-prep", runtime_tool_view());
        let engine = TurnEngine::new(4);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let prepared_intent = runtime.block_on(async {
            let autonomy_budget_state = AutonomyTurnBudgetState::default();
            engine
                .prepare_tool_intent(
                    &intent,
                    0,
                    &session_context,
                    &DefaultAppToolDispatcher::runtime(),
                    ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                    &autonomy_budget_state,
                    None,
                )
                .await
                .expect("tool.search request should prepare successfully")
        });

        let expected_capabilities = serde_json::to_value(
            harness
                .kernel_ctx
                .token
                .allowed_capabilities
                .iter()
                .copied()
                .collect::<Vec<_>>(),
        )
        .expect("serialize granted capabilities");

        assert_eq!(prepared_intent.request.tool_name, "tool.search");
        assert_eq!(
            prepared_intent.request.payload[crate::tools::TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD],
            expected_capabilities
        );
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_TOKEN_ID_FIELD)
                .is_none()
        );
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_SESSION_ID_FIELD)
                .is_none()
        );
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_TURN_ID_FIELD)
                .is_none()
        );
    }

    #[test]
    fn prepare_tool_intent_injects_kernel_request_prep_for_tool_invoke() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"],
            }),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "session-tool-invoke-prep".to_owned(),
            turn_id: "turn-tool-invoke-prep".to_owned(),
            tool_call_id: "call-tool-invoke-prep".to_owned(),
        };
        let session_context =
            SessionContext::root_with_tool_view("session-tool-invoke-prep", runtime_tool_view());
        let engine = TurnEngine::new(4);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let prepared_intent = runtime.block_on(async {
            let autonomy_budget_state = AutonomyTurnBudgetState::default();
            engine
                .prepare_tool_intent(
                    &intent,
                    0,
                    &session_context,
                    &DefaultAppToolDispatcher::runtime(),
                    ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                    &autonomy_budget_state,
                    None,
                )
                .await
                .expect("tool.invoke request should prepare successfully")
        });

        assert_eq!(prepared_intent.request.tool_name, "shell.exec");
        assert_eq!(
            prepared_intent.decision.tool_name, "shell.exec",
            "shell.exec should still rebind to the inner tool for execution metadata"
        );
        assert_eq!(
            prepared_intent.intent.tool_name, "shell.exec",
            "tool.invoke shell requests should continue using inner tool intent metadata"
        );
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn prepare_tool_intent_uses_inner_parallel_safe_metadata_for_tool_invoke_file_read_requests() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
            "file.read",
            json!({
                "path": "README.md",
            }),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "session-file-read-invoke-trace".to_owned(),
            turn_id: "turn-file-read-invoke-trace".to_owned(),
            tool_call_id: "call-file-read-invoke-trace".to_owned(),
        };
        let session_context = SessionContext::root_with_tool_view(
            "session-file-read-invoke-trace",
            runtime_tool_view(),
        );
        let engine = TurnEngine::new(4);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let prepared_intent = runtime.block_on(async {
            let autonomy_budget_state = AutonomyTurnBudgetState::default();
            engine
                .prepare_tool_intent(
                    &intent,
                    0,
                    &session_context,
                    &DefaultAppToolDispatcher::runtime(),
                    ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                    &autonomy_budget_state,
                    None,
                )
                .await
                .expect("tool.invoke file.read request should prepare successfully")
        });

        let request_payload = &prepared_intent.request.payload;
        let observed_tool_id = request_payload
            .get("tool_id")
            .cloned()
            .expect("tool.invoke request should preserve tool_id");
        let observed_arguments = request_payload
            .get("arguments")
            .cloned()
            .expect("tool.invoke request should preserve nested arguments");
        let expected_tool_id = Value::String("file.read".to_owned());
        let expected_arguments = json!({
            "path": "README.md",
        });

        assert_eq!(prepared_intent.request.tool_name, "tool.invoke");
        assert_eq!(observed_tool_id, expected_tool_id);
        assert_eq!(observed_arguments, expected_arguments);
        assert_eq!(prepared_intent.intent.tool_name, "file.read");
        assert_eq!(
            prepared_intent.capability_action_class,
            crate::tools::CapabilityActionClass::ExecuteExisting
        );
        assert_eq!(
            prepared_intent.scheduling_class,
            crate::tools::ToolSchedulingClass::ParallelSafe
        );
    }

    #[cfg(feature = "tool-file")]
    #[test]
    fn prepare_tool_intent_keeps_followup_tool_invoke_wrapper_unbound_to_current_turn() {
        use crate::test_support::TurnTestHarness;

        let harness = TurnTestHarness::new();
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call(
            "file.read",
            json!({
                "path": "README.md",
            }),
        );
        let intent = ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "session-tool-invoke-wrapper-prep".to_owned(),
            turn_id: "turn-tool-invoke-wrapper-prep".to_owned(),
            tool_call_id: "call-tool-invoke-wrapper-prep".to_owned(),
        };
        let session_context = SessionContext::root_with_tool_view(
            "session-tool-invoke-wrapper-prep",
            runtime_tool_view(),
        );
        let engine = TurnEngine::new(4);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let prepared_intent = runtime.block_on(async {
            let autonomy_budget_state = AutonomyTurnBudgetState::default();
            engine
                .prepare_tool_intent(
                    &intent,
                    0,
                    &session_context,
                    &DefaultAppToolDispatcher::runtime(),
                    ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
                    &autonomy_budget_state,
                    None,
                )
                .await
                .expect("tool.invoke file.read request should prepare successfully")
        });

        assert_eq!(prepared_intent.request.tool_name, "tool.invoke");
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_TOKEN_ID_FIELD)
                .is_none(),
            "conversation follow-up invoke wrapper should not be rebound to the current turn"
        );
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_SESSION_ID_FIELD)
                .is_none(),
            "conversation follow-up invoke wrapper should not be rebound to the current turn"
        );
        assert!(
            prepared_intent
                .request
                .payload
                .get(crate::tools::TOOL_LEASE_TURN_ID_FIELD)
                .is_none(),
            "conversation follow-up invoke wrapper should not be rebound to the current turn"
        );
    }

    fn delegate_async_turn(session_id: &str, turn_id: &str, tool_call_id: &str) -> ProviderTurn {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "delegate_async",
            json!({
                "task": "inspect the child task"
            }),
            Some(session_id),
            Some(turn_id),
        );
        ProviderTurn {
            assistant_text: "queueing child delegate".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name,
                args_json,
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    fn discovered_delegate_async_turn(
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
    ) -> ProviderTurn {
        delegate_async_turn(session_id, turn_id, tool_call_id)
    }

    fn external_skills_policy_get_turn(
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
    ) -> ProviderTurn {
        let payload = json!({
            "action": "get"
        });
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "external_skills.policy",
            payload,
            Some(session_id),
            Some(turn_id),
        );
        ProviderTurn {
            assistant_text: "reading external skills policy".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name,
                args_json,
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    fn discovered_shell_exec_turn(
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
    ) -> ProviderTurn {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "shell.exec",
            json!({
                "command": "cargo",
                "args": ["--version"]
            }),
            Some(session_id),
            Some(turn_id),
        );
        ProviderTurn {
            assistant_text: "checking cargo version".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name,
                args_json,
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    fn browser_companion_click_turn(
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        companion_session_id: &str,
    ) -> ProviderTurn {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "browser.companion.click",
            json!({
                "session_id": companion_session_id,
                "selector": "#submit"
            }),
            Some(session_id),
            Some(turn_id),
        );
        ProviderTurn {
            assistant_text: "clicking through browser companion".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name,
                args_json,
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    fn provider_tool_turn(
        tool_name: &str,
        args_json: serde_json::Value,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
    ) -> ProviderTurn {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            tool_name,
            args_json,
            Some(session_id),
            Some(turn_id),
        );
        ProviderTurn {
            assistant_text: format!("calling {tool_name}"),
            tool_intents: vec![ToolIntent {
                tool_name,
                args_json,
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    fn provider_app_tool_intent(
        tool_name: &str,
        args_json: serde_json::Value,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
    ) -> ToolIntent {
        let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            tool_name,
            args_json,
            Some(session_id),
            Some(turn_id),
        );
        ToolIntent {
            tool_name,
            args_json,
            source: "assistant".to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
        }
    }

    fn fast_lane_observed_execution_turn(
        session_id: &str,
        turn_id: &str,
        call_prefix: &str,
    ) -> ProviderTurn {
        ProviderTurn {
            assistant_text: "observing mixed fast-lane execution".to_owned(),
            tool_intents: vec![
                provider_app_tool_intent(
                    "sessions_list",
                    json!({}),
                    session_id,
                    turn_id,
                    &format!("{call_prefix}-1"),
                ),
                provider_app_tool_intent(
                    "sessions_list",
                    json!({}),
                    session_id,
                    turn_id,
                    &format!("{call_prefix}-2"),
                ),
                provider_app_tool_intent(
                    "session_status",
                    json!({"session_id": session_id}),
                    session_id,
                    turn_id,
                    &format!("{call_prefix}-3"),
                ),
                provider_app_tool_intent(
                    "sessions_list",
                    json!({}),
                    session_id,
                    turn_id,
                    &format!("{call_prefix}-4"),
                ),
                provider_app_tool_intent(
                    "sessions_list",
                    json!({}),
                    session_id,
                    turn_id,
                    &format!("{call_prefix}-5"),
                ),
            ],
            raw_meta: json!({}),
        }
    }

    struct DelayedObservedExecutionDispatcher;

    #[async_trait::async_trait]
    impl AppToolDispatcher for DelayedObservedExecutionDispatcher {
        async fn execute_app_tool(
            &self,
            session_context: &SessionContext,
            request: ToolCoreRequest,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> Result<ToolCoreOutcome, String> {
            let payload_delay_ms = request.payload.get("delay_ms").and_then(Value::as_u64);
            let delay_ms = match payload_delay_ms {
                Some(delay_ms) => delay_ms,
                None => match request.tool_name.as_str() {
                    "sessions_list" => 25,
                    "session_status" => 10,
                    other => return Err(format!("app_tool_not_found: {other}")),
                },
            };
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": request.tool_name,
                    "session_id": session_context.session_id,
                }),
            })
        }
    }

    struct AfterExecutionSequenceRecordingDispatcher {
        after_calls: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>>,
    }

    #[async_trait::async_trait]
    impl AppToolDispatcher for AfterExecutionSequenceRecordingDispatcher {
        async fn execute_app_tool(
            &self,
            session_context: &SessionContext,
            request: ToolCoreRequest,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> Result<ToolCoreOutcome, String> {
            let delay_ms = match request.tool_name.as_str() {
                "sessions_list" => 25,
                "session_status" => 10,
                other => return Err(format!("app_tool_not_found: {other}")),
            };
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": request.tool_name,
                    "session_id": session_context.session_id,
                }),
            })
        }

        async fn after_tool_execution(
            &self,
            _session_context: &SessionContext,
            intent: &ToolIntent,
            intent_sequence: usize,
            _request: &ToolCoreRequest,
            _outcome: &ToolCoreOutcome,
            _binding: ConversationRuntimeBinding<'_>,
        ) {
            let mut after_calls = self.after_calls.lock().expect("after call lock");
            let call_record = (intent.tool_call_id.clone(), intent_sequence);
            after_calls.push(call_record);
        }
    }

    fn partially_failing_observed_execution_turn(session_id: &str, turn_id: &str) -> ProviderTurn {
        ProviderTurn {
            assistant_text: "observing a partial tool failure".to_owned(),
            tool_intents: vec![
                provider_app_tool_intent(
                    "sessions_list",
                    json!({}),
                    session_id,
                    turn_id,
                    "call-partial-1",
                ),
                provider_app_tool_intent(
                    "session_status",
                    json!({"session_id": session_id}),
                    session_id,
                    turn_id,
                    "call-partial-2",
                ),
            ],
            raw_meta: json!({}),
        }
    }

    struct PartiallyFailingObservedExecutionDispatcher;

    #[async_trait::async_trait]
    impl AppToolDispatcher for PartiallyFailingObservedExecutionDispatcher {
        async fn execute_app_tool(
            &self,
            session_context: &SessionContext,
            request: ToolCoreRequest,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> Result<ToolCoreOutcome, String> {
            match request.tool_name.as_str() {
                "sessions_list" => Ok(ToolCoreOutcome {
                    status: "ok".to_owned(),
                    payload: json!({
                        "tool": request.tool_name,
                        "session_id": session_context.session_id,
                    }),
                }),
                "session_status" => Err("simulated observed tool failure".to_owned()),
                other => Err(format!("app_tool_not_found: {other}")),
            }
        }
    }

    fn unique_browser_companion_temp_dir(prefix: &str) -> PathBuf {
        unique_temp_dir(prefix)
    }

    #[cfg(unix)]
    fn write_browser_companion_script(
        root: &Path,
        name: &str,
        stdout_body: &str,
        log_path: &Path,
    ) -> PathBuf {
        let path = root.join(name);
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '1.2.3\\n'\n  exit 0\nfi\nBODY=''\nIFS= read -r BODY || true\nprintf '%s' \"$BODY\" > \"{}\"\nprintf '%s' '{}'\n",
            log_path.display(),
            stdout_body.replace('\'', "'\"'\"'")
        );
        crate::test_support::write_executable_script_atomically(&path, &script)
            .expect("write browser companion script");
        path
    }

    #[cfg(windows)]
    fn write_browser_companion_script(
        root: &Path,
        name: &str,
        stdout_body: &str,
        log_path: &Path,
    ) -> PathBuf {
        let path = root.join(format!("{name}.cmd"));
        let script = format!(
            "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo 1.2.3\r\n  exit /b 0\r\n)\r\nsetlocal enableextensions\r\nset /p BODY=\r\n> \"{}\" <nul set /p =%BODY%\r\necho {}\r\n",
            log_path.display(),
            stdout_body
        );
        fs::write(&path, script).expect("write browser companion script");
        path
    }

    #[tokio::test]
    async fn autonomy_policy_approval_request_is_persisted_for_delegate_async() {
        let memory_config = isolated_memory_config("persist");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = test_kernel_context("turn-engine-governed-approval-delegate-async");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &delegate_async_turn("root-session", "turn-1", "call-1"),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let approval_request_id = match result {
            TurnResult::NeedsApproval(requirement) => {
                assert_eq!(requirement.tool_name.as_deref(), Some("delegate_async"));
                assert_eq!(
                    requirement.approval_key.as_deref(),
                    Some("tool:delegate_async")
                );
                assert_eq!(
                    requirement.rule_id.as_str(),
                    "autonomy_policy_topology_mutation_requires_approval"
                );
                requirement
                    .approval_request_id
                    .expect("approval request id should be present")
            }
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected NeedsApproval, got {other:?}")
            }
        };

        let stored = repo
            .load_approval_request(&approval_request_id)
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.tool_name, "delegate_async");
        assert_eq!(stored.tool_call_id, "call-1");
        assert_eq!(stored.turn_id, "turn-1");
        assert_eq!(stored.approval_key, "tool:delegate_async");
        assert_eq!(
            stored.governance_snapshot_json["policy_source"],
            "autonomy_policy"
        );
        assert_eq!(
            stored.governance_snapshot_json["capability_action_class"],
            "topology_expand"
        );
    }

    #[tokio::test]
    async fn autonomy_policy_approval_request_is_persisted_for_discovered_delegate_async() {
        let memory_config = isolated_memory_config("persist-discovered");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx =
            test_kernel_context("turn-engine-governed-approval-discovered-delegate-async");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &discovered_delegate_async_turn(
                    "root-session",
                    "turn-discovered",
                    "call-discovered",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let approval_request_id = match result {
            TurnResult::NeedsApproval(requirement) => {
                assert_eq!(requirement.tool_name.as_deref(), Some("delegate_async"));
                assert_eq!(
                    requirement.approval_key.as_deref(),
                    Some("tool:delegate_async")
                );
                requirement
                    .approval_request_id
                    .expect("approval request id should be present")
            }
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected NeedsApproval, got {other:?}")
            }
        };

        let stored = repo
            .load_approval_request(&approval_request_id)
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.tool_name, "delegate_async");
        assert_eq!(stored.turn_id, "turn-discovered");
        assert_eq!(stored.tool_call_id, "call-discovered");
        assert_eq!(stored.approval_key, "tool:delegate_async");
        assert_eq!(stored.request_payload_json["tool_name"], "delegate_async");
        assert_eq!(
            stored.request_payload_json["args_json"],
            json!({
                "task": "inspect the child task"
            })
        );
    }

    #[tokio::test]
    async fn auto_mode_requires_approval_for_high_risk_core_tool() {
        let memory_config = isolated_memory_config("claw-migrate-core-approval");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.consent.default_mode = ToolConsentMode::Auto;
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-config-import-auto");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &provider_tool_turn(
                    "config.import",
                    json!({}),
                    "root-session",
                    "turn-config-import-auto",
                    "call-config-import-auto",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(requirement.tool_name.as_deref(), Some("config.import"));
        assert_eq!(
            requirement.approval_key.as_deref(),
            Some("tool:config.import")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "session_tool_consent_auto_blocked"
        );
        let approval_request_id = requirement
            .approval_request_id
            .expect("approval request id should be present");

        let stored = repo
            .load_approval_request(&approval_request_id)
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.tool_name, "config.import");
        assert_eq!(stored.request_payload_json["execution_kind"], "core");
    }

    #[tokio::test]
    async fn full_session_consent_skips_approval_for_high_risk_core_tool() {
        let memory_config = isolated_memory_config("claw-migrate-core-full");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        repo.upsert_session_tool_consent(crate::session::repository::NewSessionToolConsentRecord {
            scope_session_id: "root-session".to_owned(),
            mode: ToolConsentMode::Full,
            updated_by_session_id: Some("root-session".to_owned()),
        })
        .expect("persist full session consent");

        let tool_config = ToolConfig::default();
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-config-import-full");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &provider_tool_turn(
                    "config.import",
                    json!({}),
                    "root-session",
                    "turn-config-import-full",
                    "call-config-import-full",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let TurnResult::ToolError(failure) = result else {
            panic!("expected direct tool execution, got {result:?}");
        };
        assert!(
            failure
                .reason
                .contains("config.import requires payload.input_path"),
            "expected execution to reach the core tool, got: {failure:?}"
        );
    }

    #[tokio::test]
    async fn full_session_consent_still_requires_approval_for_governed_app_tool() {
        let memory_config = isolated_memory_config("browser-companion-governed-full");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        repo.upsert_session_tool_consent(crate::session::repository::NewSessionToolConsentRecord {
            scope_session_id: "root-session".to_owned(),
            mode: ToolConsentMode::Full,
            updated_by_session_id: Some("root-session".to_owned()),
        })
        .expect("persist full session consent");

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some("browser-companion".to_owned());
        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some("browser-companion".to_owned());
        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-governed-full",
                    "call-browser-companion-governed-full",
                    "browser-companion-governed-full",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(
            requirement.tool_name.as_deref(),
            Some("browser.companion.click")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "governed_tool_requires_approval"
        );
    }

    #[tokio::test]
    async fn governed_approval_allowlist_does_not_bypass_prompt_session_consent() {
        let memory_config = isolated_memory_config("browser-companion-allowlist-prompt");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.consent.default_mode = ToolConsentMode::Prompt;
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some("browser-companion".to_owned());
        tool_config
            .approval
            .approved_calls
            .push("tool:browser.companion.click".to_owned());
        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some("browser-companion".to_owned());
        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-allowlist",
                    "call-browser-companion-allowlist",
                    "browser-companion-allowlist",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(
            requirement.tool_name.as_deref(),
            Some("browser.companion.click")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "session_tool_consent_prompt_mode"
        );
    }

    #[tokio::test]
    async fn governed_approval_grant_does_not_bypass_prompt_session_consent() {
        let memory_config = isolated_memory_config("browser-companion-grant-prompt");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        repo.upsert_approval_grant(NewApprovalGrantRecord {
            scope_session_id: "root-session".to_owned(),
            approval_key: "tool:browser.companion.click".to_owned(),
            created_by_session_id: Some("root-session".to_owned()),
        })
        .expect("persist approval grant");

        let mut tool_config = ToolConfig::default();
        tool_config.consent.default_mode = ToolConsentMode::Prompt;
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some("browser-companion".to_owned());
        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some("browser-companion".to_owned());
        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-grant",
                    "call-browser-companion-grant",
                    "browser-companion-grant",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(
            requirement.tool_name.as_deref(),
            Some("browser.companion.click")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "session_tool_consent_prompt_mode"
        );
    }

    #[tokio::test]
    async fn autonomy_policy_approval_request_reuses_deterministic_id_for_same_blocked_call() {
        let memory_config = isolated_memory_config("reuse");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let turn = delegate_async_turn("root-session", "turn-reuse", "call-reuse");
        let kernel_ctx = test_kernel_context("turn-engine-governed-approval-reuse");

        let first = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;
        let second = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let first_request_id = match first {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("first approval request id"),
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected first NeedsApproval, got {other:?}")
            }
        };
        let second_request_id = match second {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("second approval request id"),
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected second NeedsApproval, got {other:?}")
            }
        };

        assert_eq!(first_request_id, second_request_id);

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].approval_request_id, first_request_id);
    }

    #[tokio::test]
    async fn autonomy_policy_preapproved_call_executes_without_persisting_request() {
        let memory_config = isolated_memory_config("preapproved");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let approval_key = "tool:external_skills.policy".to_owned();
        let mut tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        let approved_calls = &mut tool_config.approval.approved_calls;
        approved_calls.push(approval_key);

        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-autonomy-preapproved");
        let turn =
            external_skills_policy_get_turn("root-session", "turn-preapproved", "call-preapproved");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let reply = match result {
            TurnResult::FinalText(reply) => reply,
            other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected FinalText, got {other:?}")
            }
        };

        assert!(
            reply.contains("\"tool\":\"external_skills.policy\""),
            "reply should include executed external_skills.policy output: {reply}"
        );
        assert!(
            reply.contains("\"status\":\"ok\""),
            "reply should surface a successful external_skills.policy outcome: {reply}"
        );

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn autonomy_policy_predenied_call_returns_policy_denial_without_persisting_request() {
        let memory_config = isolated_memory_config("predenied");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let denial_key = "tool:external_skills.policy".to_owned();
        let mut tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        let denied_calls = &mut tool_config.approval.denied_calls;
        denied_calls.push(denial_key);

        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-autonomy-predenied");
        let turn =
            external_skills_policy_get_turn("root-session", "turn-predenied", "call-predenied");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let failure = match result {
            TurnResult::ToolDenied(failure) => failure,
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected ToolDenied, got {other:?}")
            }
        };

        assert_eq!(failure.code, "app_tool_denied");
        assert!(
            failure.reason.contains("tool:external_skills.policy"),
            "denial should reference the statically denied approval key: {failure:?}"
        );

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn governed_tool_approval_request_is_persisted_for_discovered_shell_exec() {
        let memory_config = isolated_memory_config("persist-shell");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.consent.default_mode = ToolConsentMode::Prompt;
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = bootstrap_test_kernel_context("turn-engine-governed-shell-approval", 60)
            .expect("kernel context");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &discovered_shell_exec_turn(
                    "root-session",
                    "turn-shell-discovered",
                    "call-shell-discovered",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let approval_request_id = match result {
            TurnResult::NeedsApproval(requirement) => {
                assert_eq!(requirement.tool_name.as_deref(), Some("shell.exec"));
                assert_eq!(requirement.approval_key.as_deref(), Some("tool:shell.exec"));
                requirement
                    .approval_request_id
                    .expect("approval request id should be present")
            }
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected NeedsApproval, got {other:?}")
            }
        };

        let stored = repo
            .load_approval_request(&approval_request_id)
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.tool_name, "shell.exec");
        assert_eq!(stored.turn_id, "turn-shell-discovered");
        assert_eq!(stored.tool_call_id, "call-shell-discovered");
        assert_eq!(stored.approval_key, "tool:shell.exec");
        assert_eq!(stored.request_payload_json["tool_name"], "shell.exec");
        assert_eq!(stored.request_payload_json["execution_kind"], "core");
        assert_eq!(
            stored.request_payload_json["args_json"],
            json!({
                "command": "cargo",
                "args": ["--version"]
            })
        );
    }

    #[tokio::test]
    async fn governed_tool_approval_request_reuses_deterministic_id_for_same_blocked_call() {
        let memory_config = isolated_memory_config("reuse-shell");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.consent.default_mode = ToolConsentMode::Prompt;

        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = bootstrap_test_kernel_context("turn-engine-governed-shell-reuse", 60)
            .expect("kernel context");
        let turn = discovered_shell_exec_turn("root-session", "turn-reuse", "call-reuse");

        let first = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;
        let second = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let first_request_id = match first {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("first approval request id"),
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected first NeedsApproval, got {other:?}")
            }
        };
        let second_request_id = match second {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("second approval request id"),
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected second NeedsApproval, got {other:?}")
            }
        };

        assert_eq!(first_request_id, second_request_id);

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].approval_request_id, first_request_id);
    }

    #[tokio::test]
    async fn autonomy_policy_allowlist_does_not_bypass_prompt_session_consent() {
        let memory_config = isolated_memory_config("autonomy-allowlist-prompt");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        tool_config.consent.default_mode = ToolConsentMode::Prompt;
        let approved_calls = &mut tool_config.approval.approved_calls;
        approved_calls.push("tool:external_skills.policy".to_owned());

        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-autonomy-allowlist-prompt");
        let turn = external_skills_policy_get_turn(
            "root-session",
            "turn-autonomy-allowlist-prompt",
            "call-autonomy-allowlist-prompt",
        );

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(
            requirement.tool_name.as_deref(),
            Some("external_skills.policy")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "session_tool_consent_prompt_mode"
        );

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].tool_name, "external_skills.policy");
        assert_eq!(requests[0].approval_key, "tool:external_skills.policy");
    }

    #[tokio::test]
    async fn autonomy_policy_grant_does_not_bypass_prompt_session_consent() {
        let memory_config = isolated_memory_config("autonomy-grant-prompt");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
        repo.upsert_approval_grant(NewApprovalGrantRecord {
            scope_session_id: "root-session".to_owned(),
            approval_key: "tool:external_skills.policy".to_owned(),
            created_by_session_id: Some("root-session".to_owned()),
        })
        .expect("persist approval grant");

        let mut tool_config = ToolConfig {
            autonomy_profile: AutonomyProfile::GuidedAcquisition,
            ..ToolConfig::default()
        };
        tool_config.consent.default_mode = ToolConsentMode::Prompt;

        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = kernel_context("turn-engine-autonomy-grant-prompt");
        let turn = external_skills_policy_get_turn(
            "root-session",
            "turn-autonomy-grant-prompt",
            "call-autonomy-grant-prompt",
        );

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let TurnResult::NeedsApproval(requirement) = result else {
            panic!("expected NeedsApproval, got {result:?}");
        };
        assert_eq!(
            requirement.tool_name.as_deref(),
            Some("external_skills.policy")
        );
        assert_eq!(
            requirement.rule_id.as_str(),
            "session_tool_consent_prompt_mode"
        );

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].tool_name, "external_skills.policy");
        assert_eq!(requests[0].approval_key, "tool:external_skills.policy");
    }

    #[tokio::test]
    async fn governed_tool_approval_request_is_persisted_for_browser_companion_click() {
        let memory_config = isolated_memory_config("browser-companion-click-approval");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some("browser-companion".to_owned());

        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some("browser-companion".to_owned());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let kernel_ctx = test_kernel_context("turn-engine-browser-companion-click-approval");

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion",
                    "call-browser-companion",
                    "browser-companion-123",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let approval_request_id = match result {
            TurnResult::NeedsApproval(requirement) => {
                assert_eq!(
                    requirement.tool_name.as_deref(),
                    Some("browser.companion.click")
                );
                assert_eq!(
                    requirement.approval_key.as_deref(),
                    Some("tool:browser.companion.click")
                );
                requirement
                    .approval_request_id
                    .expect("approval request id should exist")
            }
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_) => {
                panic!("expected NeedsApproval, got {other:?}")
            }
        };

        let stored = repo
            .load_approval_request(&approval_request_id)
            .expect("load approval request")
            .expect("approval request row");
        assert_eq!(stored.status, ApprovalRequestStatus::Pending);
        assert_eq!(stored.tool_name, "browser.companion.click");
        assert_eq!(
            stored.request_payload_json["args_json"]["selector"],
            "#submit"
        );
    }

    #[tokio::test]
    async fn governed_tool_predenied_reason_omits_internal_prefix() {
        let memory_config = isolated_memory_config("browser-companion-click-predenied");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some("browser-companion".to_owned());
        tool_config
            .approval
            .denied_calls
            .push("tool:browser.companion.click".to_owned());

        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some("browser-companion".to_owned());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-predenied",
                    "call-browser-companion-predenied",
                    "browser-companion-123",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
            )
            .await;

        let failure = match result {
            TurnResult::ToolDenied(failure) => failure,
            other @ TurnResult::FinalText(_)
            | other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected ToolDenied, got {other:?}")
            }
        };

        assert_eq!(failure.code, "app_tool_denied");
        assert!(
            !failure.reason.starts_with("app_tool_denied:"),
            "human-facing denial reason should not expose the transport prefix: {failure:?}"
        );
        assert!(
            failure.reason.contains("tool:browser.companion.click"),
            "denial should still identify the governed tool: {failure:?}"
        );

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert!(requests.is_empty());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn browser_companion_click_turn_executes_when_approval_is_disabled() {
        let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
        let mut env = crate::test_support::ScopedEnv::new();
        let memory_config = isolated_memory_config("browser-companion-click-exec");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let root = unique_browser_companion_temp_dir("loong-turn-engine-browser-companion");
        fs::create_dir_all(&root).expect("create fixture root");
        let log_path = root.join("request.json");
        let script_path = write_browser_companion_script(
            &root,
            "browser-companion-click",
            r#"{"ok":true,"result":{"clicked":true}}"#,
            &log_path,
        );

        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some(script_path.display().to_string());

        let start = crate::tools::execute_tool_core_with_config(
            loong_contracts::ToolCoreRequest {
                tool_name: "browser.companion.session.start".to_owned(),
                payload: json!({
                    "url": "https://example.com",
                    crate::tools::BROWSER_SESSION_SCOPE_FIELD: "root-session"
                }),
            },
            &runtime_config,
        )
        .expect("browser companion start should succeed");
        let companion_session_id = start.payload["session_id"]
            .as_str()
            .expect("session id should exist")
            .to_owned();

        env.set("LOONG_BROWSER_COMPANION_READY", "true");

        let mut tool_config = ToolConfig::default();
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some(script_path.display().to_string());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, tool_config);
        let kernel_ctx = test_kernel_context("turn-engine-browser-companion-click-exec");
        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-exec",
                    "call-browser-companion-exec",
                    &companion_session_id,
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let reply = match result {
            TurnResult::FinalText(reply) => reply,
            other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected FinalText, got {other:?}")
            }
        };
        assert!(
            reply.contains("\"tool\":\"browser\""),
            "reply should include the executed browser surface output: {reply}"
        );
        assert!(
            reply.contains("\"status\":\"ok\""),
            "reply should show a successful tool outcome: {reply}"
        );

        let request: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&log_path).expect("request log should exist"))
                .expect("request log should be valid json");
        assert_eq!(request["session_scope"], "root-session");
        assert_eq!(request["operation"], "click");

        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn browser_companion_click_turn_uses_runtime_visible_readiness_without_env_recheck() {
        let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
        let mut env = crate::test_support::ScopedEnv::new();
        let memory_config = isolated_memory_config("browser-companion-click-runtime-ready");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let root = unique_browser_companion_temp_dir("loong-turn-engine-browser-companion-runtime");
        fs::create_dir_all(&root).expect("create fixture root");
        let log_path = root.join("request.json");
        let script_path = write_browser_companion_script(
            &root,
            "browser-companion-click-runtime",
            r#"{"ok":true,"result":{"clicked":true}}"#,
            &log_path,
        );

        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some(script_path.display().to_string());

        let start = crate::tools::execute_tool_core_with_config(
            loong_contracts::ToolCoreRequest {
                tool_name: "browser.companion.session.start".to_owned(),
                payload: json!({
                    "url": "https://example.com",
                    crate::tools::BROWSER_SESSION_SCOPE_FIELD: "root-session"
                }),
            },
            &runtime_config,
        )
        .expect("browser companion start should succeed");
        let companion_session_id = start.payload["session_id"]
            .as_str()
            .expect("session id should exist")
            .to_owned();

        env.set("LOONG_BROWSER_COMPANION_READY", "false");

        let mut tool_config = ToolConfig::default();
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some(script_path.display().to_string());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, tool_config);
        let kernel_ctx = test_kernel_context("turn-engine-browser-companion-click-runtime-ready");
        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-runtime",
                    "call-browser-companion-runtime",
                    &companion_session_id,
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let reply = match result {
            TurnResult::FinalText(reply) => reply,
            other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected FinalText, got {other:?}")
            }
        };
        assert!(
            reply.contains("\"tool\":\"browser\""),
            "reply should include the executed browser surface output: {reply}"
        );
        assert!(
            reply.contains("\"status\":\"ok\""),
            "reply should show a successful tool outcome: {reply}"
        );

        let request: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&log_path).expect("request log should exist"))
                .expect("request log should be valid json");
        assert_eq!(request["session_scope"], "root-session");
        assert_eq!(request["operation"], "click");

        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn browser_companion_click_turn_uses_runtime_visible_policy_when_app_config_is_default() {
        let _subprocess_guard = crate::test_support::acquire_subprocess_test_guard();
        let mut env = crate::test_support::ScopedEnv::new();
        let memory_config = isolated_memory_config("browser-companion-click-runtime-policy");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");

        let root =
            unique_browser_companion_temp_dir("loong-turn-engine-browser-companion-runtime-policy");
        fs::create_dir_all(&root).expect("create fixture root");
        let log_path = root.join("request.json");
        let script_path = write_browser_companion_script(
            &root,
            "browser-companion-click-runtime-policy",
            r#"{"ok":true,"result":{"clicked":true}}"#,
            &log_path,
        );

        let mut runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
        runtime_config.browser_companion.enabled = true;
        runtime_config.browser_companion.ready = true;
        runtime_config.browser_companion.command = Some(script_path.display().to_string());

        let start = crate::tools::execute_tool_core_with_config(
            loong_contracts::ToolCoreRequest {
                tool_name: "browser.companion.session.start".to_owned(),
                payload: json!({
                    "url": "https://example.com",
                    crate::tools::BROWSER_SESSION_SCOPE_FIELD: "root-session"
                }),
            },
            &runtime_config,
        )
        .expect("browser companion start should succeed");
        let companion_session_id = start.payload["session_id"]
            .as_str()
            .expect("session id should exist")
            .to_owned();

        env.set("LOONG_BROWSER_COMPANION_ENABLED", "true");
        env.set("LOONG_BROWSER_COMPANION_READY", "false");
        env.set(
            "LOONG_BROWSER_COMPANION_COMMAND",
            script_path.display().to_string(),
        );

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, ToolConfig::default());
        let kernel_ctx = test_kernel_context("turn-engine-browser-companion-click-runtime-policy");
        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &browser_companion_click_turn(
                    "root-session",
                    "turn-browser-companion-runtime-policy",
                    "call-browser-companion-runtime-policy",
                    &companion_session_id,
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
                None,
            )
            .await;

        let reply = match result {
            TurnResult::FinalText(reply) => reply,
            other @ TurnResult::NeedsApproval(_)
            | other @ TurnResult::ToolDenied(_)
            | other @ TurnResult::ToolError(_)
            | other @ TurnResult::ProviderError(_)
            | other @ TurnResult::StreamingText(_)
            | other @ TurnResult::StreamingDone(_) => {
                panic!("expected FinalText, got {other:?}")
            }
        };
        assert!(
            reply.contains("\"tool\":\"browser\""),
            "reply should include the executed browser surface output: {reply}"
        );
        assert!(
            reply.contains("\"status\":\"ok\""),
            "reply should show a successful tool outcome: {reply}"
        );

        let request: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&log_path).expect("request log should exist"))
                .expect("request log should be valid json");
        assert_eq!(request["session_scope"], "root-session");
        assert_eq!(request["operation"], "click");

        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn observed_fast_lane_execution_trace_records_batch_and_segment_metrics() {
        let turn = fast_lane_observed_execution_turn(
            "session-observed-fast-lane",
            "turn-observed-fast-lane",
            "call-observed-fast-lane",
        );
        let session_context =
            SessionContext::root_with_tool_view("session-observed-fast-lane", runtime_tool_view());
        let dispatcher = DelayedObservedExecutionDispatcher;
        let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

        let (result, trace) = engine
            .execute_turn_in_context_with_trace(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;

        assert!(
            matches!(result, TurnResult::FinalText(_)),
            "expected FinalText, got {result:?}"
        );

        let trace = trace.expect("trace should exist");
        assert_eq!(trace.total_intents, 5);
        assert!(trace.parallel_execution_enabled);
        assert_eq!(trace.parallel_execution_max_in_flight, 2);
        assert_eq!(trace.observed_peak_in_flight, 2);
        assert!(
            trace.observed_wall_time_ms >= 40,
            "expected batch wall time to reflect execution, got {}",
            trace.observed_wall_time_ms
        );
        assert_eq!(trace.segments.len(), 3);
        assert_eq!(
            trace.segments[0].execution_mode,
            ToolBatchExecutionMode::Parallel
        );
        assert_eq!(trace.segments[0].observed_peak_in_flight, Some(2));
        assert!(
            trace.segments[0]
                .observed_wall_time_ms
                .expect("parallel segment wall time")
                >= 20
        );
        assert_eq!(
            trace.segments[1].execution_mode,
            ToolBatchExecutionMode::Sequential
        );
        assert_eq!(trace.segments[1].observed_peak_in_flight, Some(1));
        assert_eq!(
            trace.segments[2].execution_mode,
            ToolBatchExecutionMode::Parallel
        );
        assert_eq!(trace.segments[2].observed_peak_in_flight, Some(2));
    }

    #[tokio::test]
    async fn parallel_execution_reports_global_intent_sequence_to_after_tool_execution() {
        let turn = fast_lane_observed_execution_turn(
            "session-observed-sequence",
            "turn-observed-sequence",
            "call-observed-sequence",
        );
        let session_context =
            SessionContext::root_with_tool_view("session-observed-sequence", runtime_tool_view());
        let after_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let dispatcher = AfterExecutionSequenceRecordingDispatcher {
            after_calls: std::sync::Arc::clone(&after_calls),
        };
        let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

        let (result, _trace) = engine
            .execute_turn_in_context_with_trace(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;

        assert!(
            matches!(result, TurnResult::FinalText(_)),
            "expected FinalText, got {result:?}"
        );

        let after_calls = after_calls.lock().expect("after call lock");
        let after_call_map = after_calls
            .iter()
            .cloned()
            .collect::<std::collections::BTreeMap<String, usize>>();

        assert_eq!(after_call_map.len(), 5);
        assert_eq!(after_call_map.get("call-observed-sequence-1"), Some(&0));
        assert_eq!(after_call_map.get("call-observed-sequence-2"), Some(&1));
        assert_eq!(after_call_map.get("call-observed-sequence-3"), Some(&2));
        assert_eq!(after_call_map.get("call-observed-sequence-4"), Some(&3));
        assert_eq!(after_call_map.get("call-observed-sequence-5"), Some(&4));
    }

    #[tokio::test]
    async fn observed_fast_lane_execution_treats_single_in_flight_batches_as_sequential() {
        let turn = fast_lane_observed_execution_turn(
            "session-observed-fast-lane-single",
            "turn-observed-fast-lane-single",
            "call-observed-fast-lane-single",
        );
        let session_context = SessionContext::root_with_tool_view(
            "session-observed-fast-lane-single",
            runtime_tool_view(),
        );
        let dispatcher = DelayedObservedExecutionDispatcher;
        let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 1);

        let (_result, trace) = engine
            .execute_turn_in_context_with_trace(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;

        let trace = trace.expect("trace should exist");
        assert_eq!(trace.parallel_execution_max_in_flight, 1);
        assert_eq!(
            trace
                .segments
                .iter()
                .filter(|segment| segment.execution_mode == ToolBatchExecutionMode::Parallel)
                .count(),
            0
        );
        assert!(
            trace
                .segments
                .iter()
                .all(|segment| segment.execution_mode == ToolBatchExecutionMode::Sequential)
        );
    }

    #[tokio::test]
    async fn parallel_execution_records_trace_items_in_intent_order() {
        let turn = ProviderTurn {
            assistant_text: "observing ordered trace records".to_owned(),
            tool_intents: vec![
                provider_app_tool_intent(
                    "sessions_list",
                    json!({"delay_ms": 25}),
                    "session-observed-trace-order",
                    "turn-observed-trace-order",
                    "call-observed-trace-order-1",
                ),
                provider_app_tool_intent(
                    "sessions_list",
                    json!({"delay_ms": 5}),
                    "session-observed-trace-order",
                    "turn-observed-trace-order",
                    "call-observed-trace-order-2",
                ),
            ],
            raw_meta: json!({}),
        };
        let session_context = SessionContext::root_with_tool_view(
            "session-observed-trace-order",
            runtime_tool_view(),
        );
        let dispatcher = DelayedObservedExecutionDispatcher;
        let engine = TurnEngine::with_parallel_tool_execution(8, 512, true, 2);

        let (result, trace) = engine
            .execute_turn_in_context_with_trace(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;

        assert!(
            matches!(result, TurnResult::FinalText(_)),
            "expected FinalText, got {result:?}"
        );

        let trace = trace.expect("trace should exist");
        let intent_outcome_ids = trace
            .intent_outcomes
            .iter()
            .map(|intent_outcome| intent_outcome.tool_call_id.as_str())
            .collect::<Vec<_>>();
        let outcome_record_ids = trace
            .outcome_records
            .iter()
            .map(|outcome_record| outcome_record.tool_call_id.as_str())
            .collect::<Vec<_>>();
        let expected_ids = vec!["call-observed-trace-order-1", "call-observed-trace-order-2"];

        assert_eq!(intent_outcome_ids, expected_ids);
        assert_eq!(outcome_record_ids, expected_ids);
    }

    #[tokio::test]
    async fn observed_fast_lane_execution_trace_records_partial_tool_failure_outcomes() {
        let turn = partially_failing_observed_execution_turn(
            "session-observed-partial-failure",
            "turn-observed-partial-failure",
        );
        let session_context = SessionContext::root_with_tool_view(
            "session-observed-partial-failure",
            runtime_tool_view(),
        );
        let dispatcher = PartiallyFailingObservedExecutionDispatcher;
        let engine = TurnEngine::with_parallel_tool_execution(4, 512, false, 1);

        let (result, trace) = engine
            .execute_turn_in_context_with_trace(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
                None,
            )
            .await;

        assert!(
            matches!(result, TurnResult::ToolError(_)),
            "expected ToolError, got {result:?}"
        );

        let trace = trace.expect("trace should exist");
        assert_eq!(trace.intent_outcomes.len(), 2);
        assert_eq!(
            trace.intent_outcomes[0].status,
            ToolBatchExecutionIntentStatus::Completed
        );
        assert_eq!(trace.intent_outcomes[0].tool_call_id, "call-partial-1");
        assert_eq!(
            trace.intent_outcomes[1].status,
            ToolBatchExecutionIntentStatus::Failed
        );
        assert_eq!(trace.intent_outcomes[1].tool_call_id, "call-partial-2");
        assert!(
            trace.intent_outcomes[1]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("simulated observed tool failure")),
            "expected failure detail in trace, got {:?}",
            trace.intent_outcomes[1].detail
        );
    }

    #[test]
    fn success_outcome_trace_record_bounds_large_payloads() {
        let intent = provider_app_tool_intent(
            "file.read",
            json!({"path": "note.md"}),
            "session-bounded-payload",
            "turn-bounded-payload",
            "call-bounded-payload",
        );
        let large_payload = json!({
            "contents": "x".repeat(TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS + 128),
        });
        let outcome = ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: large_payload,
        };

        let record = build_success_tool_outcome_trace_record(&intent, &outcome);

        assert_eq!(record.outcome.tool_name, "read");
        assert_eq!(record.outcome.status, "ok");
        assert_eq!(record.turn_id, "turn-bounded-payload");
        assert_eq!(record.tool_call_id, "call-bounded-payload");
        assert_eq!(record.outcome.payload["payload_truncated"], json!(true));
        let payload_summary = record.outcome.payload["payload_summary"]
            .as_str()
            .expect("expected truncated payload summary");
        let payload_chars = record.outcome.payload["payload_chars"]
            .as_u64()
            .expect("expected original payload char count");
        assert!(
            payload_summary.len() < payload_chars as usize,
            "expected bounded payload summary, got {:?}",
            record.outcome.payload
        );
        assert!(
            payload_chars > TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS as u64,
            "expected original payload char count, got {:?}",
            record.outcome.payload
        );
    }

    #[test]
    fn continuation_payload_summary_is_compacted_before_low_limit_truncation() {
        let intent = provider_app_tool_intent(
            "session_wait",
            json!({"session_id": "child-session"}),
            "session-continuation-payload",
            "turn-continuation-payload",
            "call-continuation-payload",
        );
        let outcome = ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "session": {
                    "session_id": "child-session",
                    "state": "running",
                    "label": "Child",
                    "kind": "delegate_child",
                    "details": "x".repeat(512),
                },
                "wait_status": "waiting",
                "events": [
                    {
                        "event_kind": "delegate_result",
                        "payload": "x".repeat(512),
                    }
                ],
                "continuation": {
                    "state": "waiting",
                    "is_terminal": false,
                    "recommended_tool": "session_wait",
                    "recommended_payload": {
                        "session_id": "child-session",
                        "timeout_ms": 1000,
                    },
                    "note": "Keep waiting before presenting final completion.",
                }
            }),
        };

        let envelope = build_tool_result_envelope(&intent, &outcome, 256);
        let payload_summary =
            serde_json::from_str::<Value>(envelope.payload_summary.as_str()).expect("payload json");

        assert!(!envelope.payload_truncated, "envelope: {envelope:?}");
        assert_eq!(payload_summary["continuation"]["state"], "waiting");
        assert_eq!(
            payload_summary["continuation"]["recommended_tool"],
            "session_wait"
        );
        assert!(
            payload_summary.as_object().is_some_and(|object| {
                object.contains_key("continuation") && !object.contains_key("events")
            }),
            "compacted summary should keep only compact continuation-safe fields: {payload_summary:?}"
        );
    }

    #[test]
    fn augment_tool_payload_injects_browser_scope_for_companion_tool_invoke() {
        let (tool_name, payload) = crate::tools::synthesize_test_provider_tool_call_with_scope(
            "browser.companion.session.start",
            json!({
                "url": "https://example.com"
            }),
            Some("root-session"),
            Some("turn-browser-companion-start"),
        );

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(std::iter::empty::<&str>()),
        );
        let augmented = augment_tool_payload_for_kernel(
            &tool_name,
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload["tool_id"],
            "browser.companion.session.start"
        );
        assert_eq!(
            augmented.payload["arguments"][crate::tools::BROWSER_SESSION_SCOPE_FIELD],
            "root-session"
        );
    }

    #[test]
    fn augment_tool_payload_injects_visible_tool_ids_for_tool_search() {
        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.search", "tool.invoke", "file.read"]),
        )
        .with_runtime_narrowing(crate::tools::runtime_config::ToolRuntimeNarrowing {
            browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                max_sessions: Some(1),
                ..crate::tools::runtime_config::BrowserRuntimeNarrowing::default()
            },
            ..crate::tools::runtime_config::ToolRuntimeNarrowing::default()
        });
        let payload = json!({
            "query": "read note.md",
            "limit": 3,
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.search",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_TOOL_SEARCH_KEY]
                [crate::tools::LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY],
            json!(["file.read", "tool.invoke", "tool.search"])
        );
        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_RUNTIME_NARROWING_KEY]["browser"]["max_sessions"],
            1
        );
    }

    #[test]
    fn augment_tool_payload_uses_active_skill_root_for_absolute_file_read_targets() {
        let workspace_root =
            crate::test_support::unique_temp_dir("turn-engine-active-skill-workspace");
        let skill_root = workspace_root.join(".loong/skills/demo-skill");
        std::fs::create_dir_all(skill_root.join("references")).expect("create skill root");
        let reference_path = skill_root.join("references/guide.md");
        std::fs::write(&reference_path, "# Guide\n").expect("write guide");
        let canonical_skill_root =
            std::fs::canonicalize(&skill_root).expect("canonical skill root");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke"]),
        )
        .with_workspace_root(workspace_root)
        .with_active_external_skill_roots(vec![skill_root]);
        let payload = json!({
            "tool_id": "file.read",
            "lease": "lease-file-read",
            "arguments": {
                "path": reference_path.display().to_string()
            },
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
            json!(canonical_skill_root.display().to_string())
        );
    }

    #[test]
    fn augment_tool_payload_uses_visible_skill_root_for_absolute_skill_file_reads() {
        let workspace_root =
            crate::test_support::unique_temp_dir("turn-engine-visible-skill-workspace");
        let skill_root = workspace_root.join(".loong/skills/demo-skill");
        std::fs::create_dir_all(&skill_root).expect("create skill root");
        let skill_path = skill_root.join("SKILL.md");
        std::fs::write(&skill_path, "# Demo Skill\n").expect("write skill file");
        let canonical_skill_root =
            std::fs::canonicalize(&skill_root).expect("canonical skill root");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke"]),
        )
        .with_workspace_root(workspace_root)
        .with_visible_external_skill_roots(vec![skill_root]);
        let payload = json!({
            "tool_id": "file.read",
            "lease": "lease-file-read",
            "arguments": {
                "path": skill_path.display().to_string()
            },
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
            json!(canonical_skill_root.display().to_string())
        );
    }

    #[test]
    fn augment_tool_payload_uses_visible_skill_root_for_absolute_skill_resource_reads() {
        let workspace_root =
            crate::test_support::unique_temp_dir("turn-engine-visible-skill-resource-workspace");
        let skill_root = workspace_root.join(".loong/skills/demo-skill");
        std::fs::create_dir_all(skill_root.join("references")).expect("create skill root");
        let reference_path = skill_root.join("references/guide.md");
        std::fs::write(&reference_path, "# Guide\n").expect("write guide");
        let canonical_skill_root =
            std::fs::canonicalize(&skill_root).expect("canonical skill root");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke"]),
        )
        .with_workspace_root(workspace_root)
        .with_visible_external_skill_roots(vec![skill_root]);
        let payload = json!({
            "tool_id": "file.read",
            "lease": "lease-file-read",
            "arguments": {
                "path": reference_path.display().to_string()
            },
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
            json!(canonical_skill_root.display().to_string())
        );
    }

    #[test]
    fn augment_tool_payload_uses_unique_active_skill_root_for_relative_file_read_targets() {
        let workspace_root =
            crate::test_support::unique_temp_dir("turn-engine-active-skill-relative-workspace");
        let first_skill_root = workspace_root.join(".loong/skills/demo-skill");
        let second_skill_root = workspace_root.join(".loong/skills/other-skill");
        std::fs::create_dir_all(first_skill_root.join("references")).expect("create first skill");
        std::fs::create_dir_all(second_skill_root.join("references")).expect("create second skill");
        std::fs::write(first_skill_root.join("references/guide.md"), "# First\n")
            .expect("write first guide");
        std::fs::write(second_skill_root.join("references/other.md"), "# Second\n")
            .expect("write second guide");
        let canonical_first_skill_root =
            std::fs::canonicalize(&first_skill_root).expect("canonical first skill root");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke"]),
        )
        .with_workspace_root(workspace_root)
        .with_active_external_skill_roots(vec![first_skill_root, second_skill_root]);
        let payload = json!({
            "tool_id": "file.read",
            "lease": "lease-file-read",
            "arguments": {
                "path": "references/guide.md"
            },
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
            json!(canonical_first_skill_root.display().to_string())
        );
    }

    #[test]
    fn augment_tool_payload_does_not_guess_when_relative_file_read_matches_multiple_skill_roots() {
        let workspace_root =
            crate::test_support::unique_temp_dir("turn-engine-active-skill-relative-ambiguous");
        let first_skill_root = workspace_root.join(".loong/skills/demo-skill");
        let second_skill_root = workspace_root.join(".loong/skills/other-skill");
        std::fs::create_dir_all(first_skill_root.join("references")).expect("create first skill");
        std::fs::create_dir_all(second_skill_root.join("references")).expect("create second skill");
        std::fs::write(first_skill_root.join("references/shared.md"), "# First\n")
            .expect("write first guide");
        std::fs::write(second_skill_root.join("references/shared.md"), "# Second\n")
            .expect("write second guide");
        let expected_workspace_root = workspace_root.display().to_string();

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke"]),
        )
        .with_workspace_root(workspace_root)
        .with_active_external_skill_roots(vec![first_skill_root, second_skill_root]);
        let payload = json!({
            "tool_id": "file.read",
            "lease": "lease-file-read",
            "arguments": {
                "path": "references/shared.md"
            },
        });

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            payload,
            &session_context,
            &SessionStoreConfig::default(),
        );

        assert_eq!(
            augmented.payload[crate::tools::LOONG_INTERNAL_TOOL_CONTEXT_KEY]
                [crate::tools::LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
            json!(expected_workspace_root)
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn augment_tool_payload_injects_canonical_task_id_for_task_tools() {
        let memory_config = isolated_memory_config("task-tool-scope");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Running,
        })
        .expect("create session");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: None,
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["task_wait"]),
        );

        let augmented = augment_tool_payload_for_kernel(
            "task_wait",
            json!({}),
            &session_context,
            &memory_config,
        );

        assert_eq!(augmented.payload["task_id"], "task-root");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn augment_tool_payload_injects_canonical_task_id_for_task_events() {
        let memory_config = isolated_memory_config("task-events-tool-scope");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Running,
        })
        .expect("create session");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: None,
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["task_events"]),
        );

        let augmented = augment_tool_payload_for_kernel(
            "task_events",
            json!({}),
            &session_context,
            &memory_config,
        );

        assert_eq!(augmented.payload["task_id"], "task-root");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn augment_tool_payload_injects_canonical_task_id_for_nested_task_invoke() {
        let memory_config = isolated_memory_config("task-tool-invoke-scope");
        let repo = SessionRepository::new(&memory_config).expect("repository");
        repo.ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Running,
        })
        .expect("create session");
        repo.append_event(NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: crate::task_progress::task_progress_event_payload(
                "unit_test",
                &crate::task_progress::TaskProgressRecord {
                    task_id: "task-root".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: crate::task_progress::TaskProgressStatus::Waiting,
                    intent_summary: None,
                    verification_state: Some(crate::task_progress::TaskVerificationState::Pending),
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress");

        let session_context = SessionContext::root_with_tool_view(
            "root-session",
            crate::tools::ToolView::from_tool_names(["tool.invoke", "task_wait"]),
        );

        let augmented = augment_tool_payload_for_kernel(
            "tool.invoke",
            json!({
                "tool_id": "task_wait",
                "arguments": {}
            }),
            &session_context,
            &memory_config,
        );

        assert_eq!(augmented.payload["arguments"]["task_id"], "task-root");
    }
}
