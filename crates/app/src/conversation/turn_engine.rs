use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use loongclaw_contracts::{
    Capability, KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::config::{GovernedToolApprovalMode, LoongClawConfig, SessionVisibility, ToolConfig};
use crate::context::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewApprovalRequestRecord, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};
use crate::tools::{
    ToolApprovalMode, ToolExecutionKind, ToolView, delegate_child_tool_view_for_config,
    delegate_child_tool_view_for_config_with_delegate, runtime_tool_view,
    governance_profile_for_descriptor, runtime_tool_view_for_config, tool_catalog,
};

use super::runtime::SessionContext;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultEnvelope {
    pub status: String,
    pub tool: String,
    pub tool_call_id: String,
    pub payload_summary: String,
    pub payload_chars: usize,
    pub payload_truncated: bool,
}

const TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 2048;
const MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 256;
const MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 64_000;

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
}

impl TurnFailure {
    pub fn policy_denied(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::PolicyDenied,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
        }
    }

    pub fn retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Retryable,
            code: code.into(),
            reason: reason.into(),
            retryable: true,
        }
    }

    pub fn non_retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::NonRetryable,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
        }
    }

    pub fn provider(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Provider,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
        }
    }

    pub fn as_str(&self) -> &str {
        self.reason.as_str()
    }
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
pub enum TurnResult {
    FinalText(String),
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
            TurnResult::FinalText(_) | TurnResult::NeedsApproval(_) => None,
            TurnResult::ToolDenied(failure)
            | TurnResult::ToolError(failure)
            | TurnResult::ProviderError(failure) => Some(failure),
        }
    }
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
        KernelError::ToolPlane(ToolPlaneError::Execution(_)) => {
            KernelFailureClass::RetryableExecution
        }
        _ => KernelFailureClass::NonRetryable,
    }
}

#[async_trait]
pub trait AppToolDispatcher: Send + Sync {
    async fn maybe_require_approval(
        &self,
        _session_context: &SessionContext,
        _intent: &ToolIntent,
        _descriptor: &crate::tools::ToolDescriptor,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        Ok(None)
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String>;
}

pub struct NoopAppToolDispatcher;

#[async_trait]
impl AppToolDispatcher for NoopAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        _session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        Err(format!("app_tool_not_implemented: {}", request.tool_name))
    }
}

#[derive(Clone)]
pub struct DefaultAppToolDispatcher {
    memory_config: MemoryRuntimeConfig,
    tool_config: ToolConfig,
    app_config: Option<Arc<LoongClawConfig>>,
}

impl DefaultAppToolDispatcher {
    pub fn new(memory_config: MemoryRuntimeConfig, tool_config: ToolConfig) -> Self {
        Self {
            memory_config,
            tool_config,
            app_config: None,
        }
    }

    pub fn with_config(memory_config: MemoryRuntimeConfig, app_config: LoongClawConfig) -> Self {
        Self {
            memory_config,
            tool_config: app_config.tools.clone(),
            app_config: Some(Arc::new(app_config)),
        }
    }

    pub fn runtime() -> Self {
        Self::new(
            crate::memory::runtime_config::get_memory_runtime_config().clone(),
            ToolConfig::default(),
        )
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
                let depth = repo
                    .session_lineage_depth(&session_context.session_id)
                    .map_err(|error| {
                        format!(
                            "compute session lineage depth for dispatcher tool view failed: {error}"
                        )
                    })?;
                let allow_nested_delegate = depth < self.tool_config.delegate.max_depth;
                return Ok(delegate_child_tool_view_for_config_with_delegate(
                    &self.tool_config,
                    allow_nested_delegate,
                ));
            }
            return Ok(runtime_tool_view_for_config(&self.tool_config));
        }
        if repo
            .load_session_summary_with_legacy_fallback(&session_context.session_id)?
            .is_some_and(|session| session.kind == SessionKind::DelegateChild)
        {
            return Ok(delegate_child_tool_view_for_config(&self.tool_config));
        }
        Ok(runtime_tool_view_for_config(&self.tool_config))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    fn effective_tool_view_for_session(
        &self,
        _session_context: &SessionContext,
    ) -> Result<ToolView, String> {
        Ok(runtime_tool_view_for_config(&self.tool_config))
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
        let mut current_session_id = session_context.session_id.clone();
        let mut visited = BTreeSet::new();

        loop {
            if !visited.insert(current_session_id.clone()) {
                return Err(format!(
                    "session_lineage_cycle_detected: `{current_session_id}` reappeared while resolving approval grant scope"
                ));
            }
            let Some(session) = repo.load_session(&current_session_id)? else {
                return Ok(current_session_id);
            };
            match session.parent_session_id {
                Some(parent_session_id) => current_session_id = parent_session_id,
                None => return Ok(current_session_id),
            }
        }
    }
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
    format!("apr_{:x}", hasher.finalize())
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
    async fn maybe_require_approval(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_context, intent, descriptor);
            Ok(None)
        }

        #[cfg(feature = "memory-sqlite")]
        {
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

            let approval_key = format!("tool:{}", descriptor.name);
            if self
                .tool_config
                .approval
                .approved_calls
                .iter()
                .any(|entry| entry == &approval_key)
            {
                return Ok(None);
            }
            if self
                .tool_config
                .approval
                .denied_calls
                .iter()
                .any(|entry| entry == &approval_key)
            {
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
            if repo
                .load_approval_grant(&scope_session_id, &approval_key)?
                .is_some()
            {
                return Ok(None);
            }

            let approval_request_id =
                governed_approval_request_id(session_context, descriptor.name, intent);
            let reason = format!("operator approval required before running `{}`", descriptor.name);
            let rule_id = "governed_tool_requires_approval";
            let request_payload_json = json!({
                "session_id": session_context.session_id,
                "parent_session_id": session_context.parent_session_id,
                "turn_id": intent.turn_id,
                "tool_call_id": intent.tool_call_id,
                "tool_name": descriptor.name,
                "args_json": intent.args_json,
                "source": intent.source,
                "execution_kind": match descriptor.execution_kind {
                    ToolExecutionKind::Core => "core",
                    ToolExecutionKind::App => "app",
                },
            });
            let governance_snapshot_json = json!({
                "governance_scope": governance.scope.as_str(),
                "risk_class": governance.risk_class.as_str(),
                "approval_mode": governance.approval_mode.as_str(),
                "rule_id": rule_id,
                "reason": reason,
            });
            let stored = repo.ensure_approval_request(NewApprovalRequestRecord {
                approval_request_id: approval_request_id.clone(),
                session_id: session_context.session_id.clone(),
                turn_id: intent.turn_id.clone(),
                tool_call_id: intent.tool_call_id.clone(),
                tool_name: descriptor.name.to_owned(),
                approval_key: approval_key.clone(),
                request_payload_json,
                governance_snapshot_json,
            })?;

            Ok(Some(ApprovalRequirement::governed_tool(
                descriptor.name,
                approval_key,
                reason,
                rule_id,
                Some(stored.approval_request_id),
            )))
        }
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<ToolCoreOutcome, String> {
        let canonical_tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
        let effective_tool_view = self.effective_tool_view_for_session(session_context)?;
        if let Some(descriptor) = tool_catalog().descriptor(canonical_tool_name)
            && descriptor.execution_kind == ToolExecutionKind::App
            && (!session_context.tool_view.contains(descriptor.name)
                || !effective_tool_view.contains(descriptor.name))
        {
            return Err(format!("tool_not_visible: {}", descriptor.name));
        }

        let effective_tool_config = self.effective_tool_config_for_session(session_context);
        if canonical_tool_name == "session_wait" {
            return crate::tools::wait_for_session_with_config(
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
        crate::tools::execute_app_tool_with_config(
            request,
            &session_context.session_id,
            &self.memory_config,
            &effective_tool_config,
        )
    }
}

#[allow(dead_code)]
pub(crate) fn format_tool_result_line(intent: &ToolIntent, outcome: &ToolCoreOutcome) -> String {
    format_tool_result_line_with_limit(intent, outcome, TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS)
}

pub(crate) fn format_tool_result_line_with_limit(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
    payload_summary_limit_chars: usize,
) -> String {
    let envelope = build_tool_result_envelope(intent, outcome, payload_summary_limit_chars);
    let encoded = serde_json::to_string(&envelope).unwrap_or_else(|_| {
        format!(
            "{{\"status\":\"{}\",\"tool\":\"{}\",\"tool_call_id\":\"{}\",\"payload_summary\":\"[tool_payload_unserializable]\",\"payload_chars\":0,\"payload_truncated\":false}}",
            outcome.status,
            crate::tools::canonical_tool_name(intent.tool_name.as_str()),
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
    let normalized_limit = effective_payload_summary_limit(intent, payload_summary_limit_chars)
        .clamp(
            MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
            MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        );
    let payload_text = serde_json::to_string(&outcome.payload)
        .unwrap_or_else(|_| "[tool_payload_unserializable]".to_owned());
    let (payload_summary, payload_chars, payload_truncated) =
        truncate_by_chars(payload_text.as_str(), normalized_limit);

    ToolResultEnvelope {
        status: outcome.status.clone(),
        tool: crate::tools::canonical_tool_name(intent.tool_name.as_str()).to_owned(),
        tool_call_id: intent.tool_call_id.clone(),
        payload_summary,
        payload_chars,
        payload_truncated,
    }
}

fn effective_payload_summary_limit(intent: &ToolIntent, default_limit: usize) -> usize {
    if crate::tools::canonical_tool_name(intent.tool_name.as_str()) == "external_skills.invoke" {
        return MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS;
    }
    default_limit
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

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
    tool_result_payload_summary_limit_chars: usize,
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self::with_tool_result_payload_summary_limit(
            max_tool_steps,
            TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        )
    }

    pub fn with_tool_result_payload_summary_limit(
        max_tool_steps: usize,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            max_tool_steps,
            tool_result_payload_summary_limit_chars: tool_result_payload_summary_limit_chars.clamp(
                MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
                MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
            ),
        }
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
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::policy_denied("max_tool_steps_exceeded", "max_tool_steps_exceeded");
        }

        // Check each tool intent
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_found", reason);
            };
            if !session_context.tool_view.contains(descriptor.name) {
                let reason = format!("tool_not_visible: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_visible", reason);
            }
        }

        // All tools validated — execution requires a kernel context
        TurnResult::policy_denied("kernel_context_required", "kernel_context_required")
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. No kernel context → `ToolDenied("no_kernel_context")`
    /// 5. Policy/capability check via kernel → `ToolDenied`
    /// 6. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_view(turn, &runtime_tool_view(), kernel_ctx)
            .await
    }

    pub async fn execute_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, tool_view.clone()),
            &DefaultAppToolDispatcher::runtime(),
            kernel_ctx,
        )
        .await
    }

    pub async fn execute_turn_in_context<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        kernel_ctx: Option<&KernelContext>,
    ) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::policy_denied("max_tool_steps_exceeded", "max_tool_steps_exceeded");
        }

        // Check each tool intent is known
        let catalog = tool_catalog();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_found", reason);
            };
            if !session_context.tool_view.contains(descriptor.name) {
                let reason = format!("tool_not_visible: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_visible", reason);
            }
        }

        let mut outputs = Vec::new();
        for intent in &turn.tool_intents {
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_found", reason);
            };
            let request = ToolCoreRequest {
                tool_name: descriptor.name.to_owned(),
                payload: intent.args_json.clone(),
            };
            let outcome = match descriptor.execution_kind {
                ToolExecutionKind::Core => {
                    let ctx = match kernel_ctx {
                        Some(ctx) => ctx,
                        None => {
                            return TurnResult::policy_denied(
                                "no_kernel_context",
                                "no_kernel_context",
                            );
                        }
                    };
                    let caps = BTreeSet::from([Capability::InvokeTool]);
                    match ctx
                        .kernel
                        .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(e) => {
                            let reason = format!("{e}");
                            return match classify_kernel_error(&e) {
                                KernelFailureClass::PolicyDenied => {
                                    TurnResult::policy_denied("kernel_policy_denied", reason)
                                }
                                KernelFailureClass::RetryableExecution => {
                                    TurnResult::retryable_tool_error(
                                        "tool_execution_failed",
                                        reason,
                                    )
                                }
                                KernelFailureClass::NonRetryable => {
                                    TurnResult::non_retryable_tool_error(
                                        "kernel_execution_failed",
                                        reason,
                                    )
                                }
                            };
                        }
                    }
                }
                ToolExecutionKind::App => {
                    match app_dispatcher
                        .maybe_require_approval(session_context, intent, descriptor, kernel_ctx)
                        .await
                    {
                        Ok(Some(requirement)) => return TurnResult::NeedsApproval(requirement),
                        Ok(None) => {}
                        Err(reason) if reason.starts_with("app_tool_denied:") => {
                            return TurnResult::policy_denied("app_tool_denied", reason);
                        }
                        Err(reason) => {
                            return TurnResult::non_retryable_tool_error(
                                "app_tool_preflight_failed",
                                reason,
                            );
                        }
                    }

                    match app_dispatcher
                        .execute_app_tool(session_context, request, kernel_ctx)
                        .await
                    {
                        Ok(outcome) => outcome,
                        Err(reason) if reason.starts_with("tool_not_visible:") => {
                            return TurnResult::policy_denied("tool_not_visible", reason);
                        }
                        Err(reason)
                            if reason.starts_with("tool_not_found:")
                                || reason.starts_with("app_tool_not_found:") =>
                        {
                            return TurnResult::policy_denied("tool_not_found", reason);
                        }
                        Err(reason) if reason.starts_with("app_tool_disabled:") => {
                            return TurnResult::policy_denied("app_tool_disabled", reason);
                        }
                        Err(reason) if reason.starts_with("app_tool_denied:") => {
                            return TurnResult::policy_denied("app_tool_denied", reason);
                        }
                        Err(reason) => {
                            return TurnResult::non_retryable_tool_error(
                                "app_tool_execution_failed",
                                reason,
                            );
                        }
                    }
                }
            };

            outputs.push(format_tool_result_line_with_limit(
                intent,
                &outcome,
                self.tool_result_payload_summary_limit_chars,
            ));
        }

        TurnResult::FinalText(outputs.join("\n"))
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
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::config::{GovernedToolApprovalMode, ToolConfig};
    use crate::session::repository::{
        ApprovalRequestStatus, NewSessionRecord, SessionKind, SessionRepository, SessionState,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-turn-engine-approval-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn delegate_async_turn(session_id: &str, turn_id: &str, tool_call_id: &str) -> ProviderTurn {
        ProviderTurn {
            assistant_text: "queueing child delegate".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "delegate_async".to_owned(),
                args_json: json!({
                    "task": "inspect the child task"
                }),
                source: "assistant".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: tool_call_id.to_owned(),
            }],
            raw_meta: json!({}),
        }
    }

    #[tokio::test]
    async fn governed_tool_approval_request_is_persisted_for_delegate_async() {
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

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &delegate_async_turn("root-session", "turn-1", "call-1"),
                &session_context,
                &dispatcher,
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
                    "governed_tool_requires_approval"
                );
                requirement
                    .approval_request_id
                    .expect("approval request id should be present")
            }
            other => panic!("expected NeedsApproval, got {other:?}"),
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
    }

    #[tokio::test]
    async fn governed_tool_approval_request_reuses_deterministic_id_for_same_blocked_call() {
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

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);
        let turn = delegate_async_turn("root-session", "turn-reuse", "call-reuse");

        let first = TurnEngine::new(4)
            .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
            .await;
        let second = TurnEngine::new(4)
            .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
            .await;

        let first_request_id = match first {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("first approval request id"),
            other => panic!("expected first NeedsApproval, got {other:?}"),
        };
        let second_request_id = match second {
            TurnResult::NeedsApproval(requirement) => requirement
                .approval_request_id
                .expect("second approval request id"),
            other => panic!("expected second NeedsApproval, got {other:?}"),
        };

        assert_eq!(first_request_id, second_request_id);

        let requests = repo
            .list_approval_requests_for_session("root-session", None)
            .expect("list approval requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].approval_request_id, first_request_id);
    }
}
