use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use loongclaw_contracts::{KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
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
    ToolApprovalMode, ToolExecutionKind, ToolSchedulingClass, ToolView,
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    governance_profile_for_descriptor, runtime_tool_view, runtime_tool_view_for_config,
    tool_catalog,
};

use super::runtime::SessionContext;
use super::runtime_binding::ConversationRuntimeBinding;

use super::ingress::{ConversationIngressContext, inject_internal_tool_ingress};

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

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let kernel_ctx = binding.kernel_context();
        self.maybe_require_approval(session_context, intent, descriptor, kernel_ctx)
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, String>;
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
                return Ok(with_runtime_ready_browser_companion_tools(
                    delegate_child_tool_view_for_config_with_delegate(
                        &self.tool_config,
                        allow_nested_delegate,
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
            return Ok(with_runtime_ready_browser_companion_tools(
                delegate_child_tool_view_for_config(&self.tool_config),
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
        kernel_ctx: Option<&KernelContext>,
    ) -> Result<Option<ApprovalRequirement>, String> {
        let binding = ConversationRuntimeBinding::from_optional_kernel_context(kernel_ctx);
        self.maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
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
            let reason = format!(
                "operator approval required before running `{}`",
                descriptor.name
            );
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
                approval_request_id,
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
        _binding: ConversationRuntimeBinding<'_>,
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
) -> serde_json::Value {
    let payload = inject_runtime_narrowing_context(payload, session_context);

    // Direct browser tool calls: inject scope at the top level.
    if browser_scope_injection_required(canonical_tool_name) {
        return inject_browser_scope_field(payload, &session_context.session_id);
    }

    // tool.invoke wrapping a browser tool: inject scope into the nested arguments.
    let is_browser_invoke = canonical_tool_name == "tool.invoke"
        && payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str)
            .map(crate::tools::canonical_tool_name)
            .is_some_and(browser_scope_injection_required);
    if is_browser_invoke && let serde_json::Value::Object(mut outer) = payload {
        if let Some(arguments) = outer.remove("arguments") {
            outer.insert(
                "arguments".to_owned(),
                inject_browser_scope_field(arguments, &session_context.session_id),
            );
        }
        return serde_json::Value::Object(outer);
    }

    payload
}

fn inject_runtime_narrowing_context(
    payload: serde_json::Value,
    session_context: &SessionContext,
) -> serde_json::Value {
    let Some(runtime_narrowing) = session_context.runtime_narrowing.as_ref() else {
        return payload;
    };
    if runtime_narrowing.is_empty() {
        return payload;
    }

    let serde_json::Value::Object(mut object) = payload else {
        return payload;
    };
    let mut internal = object
        .remove(crate::tools::LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    internal.insert(
        crate::tools::LOONGCLAW_INTERNAL_RUNTIME_NARROWING_KEY.to_owned(),
        serde_json::to_value(runtime_narrowing)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new())),
    );
    object.insert(
        crate::tools::LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        serde_json::Value::Object(internal),
    );
    serde_json::Value::Object(object)
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
        tool: effective_tool_name,
        tool_call_id: intent.tool_call_id.clone(),
        payload_summary,
        payload_chars,
        payload_truncated,
    }
}

fn effective_payload_summary_limit(intent: &ToolIntent, default_limit: usize) -> usize {
    if effective_result_tool_name(intent) == "external_skills.invoke" {
        return MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS;
    }
    default_limit
}

fn effective_result_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
    if canonical_tool_name != "tool.invoke" {
        return canonical_tool_name.to_owned();
    }
    intent
        .args_json
        .get("tool_id")
        .and_then(serde_json::Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .and_then(|tool_name| {
            crate::tools::resolve_tool_execution(tool_name).map(|resolved| resolved.canonical_name)
        })
        .filter(|tool_name| !crate::tools::is_provider_exposed_tool_name(tool_name))
        .unwrap_or(canonical_tool_name)
        .to_owned()
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

    intent
        .args_json
        .get("tool_id")
        .and_then(serde_json::Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .and_then(|tool_name| {
            tool_catalog()
                .descriptor(tool_name)
                .filter(|target| !target.is_provider_core())
                .map(|target| target.name.to_owned())
        })
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

    if !descriptor.is_provider_core() {
        return true;
    }

    !tool_is_visible
        && descriptor.name == "tool.invoke"
        && effective_visible_tool_name(intent, descriptor) != descriptor.name
}

fn concealed_provider_tool_denial() -> TurnFailure {
    TurnFailure::policy_denied(
        "tool_not_found",
        "tool_not_found: requested tool is not available",
    )
}

fn tool_intent_is_visible(
    session_context: &SessionContext,
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> bool {
    if descriptor.is_provider_core() {
        if descriptor.name != "tool.invoke" {
            return true;
        }
        let effective_name = effective_visible_tool_name(intent, descriptor);
        return effective_name == descriptor.name
            || session_context.tool_view.contains(effective_name.as_str());
    }

    session_context.tool_view.contains(descriptor.name)
}

async fn execute_tool_intent_via_kernel(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
    trusted_internal_context: bool,
) -> Result<ToolCoreOutcome, TurnFailure> {
    crate::tools::execute_kernel_tool_request(kernel_ctx, request, trusted_internal_context)
        .await
        .map_err(|error| {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolBatchExecutionTrace {
    pub total_intents: usize,
    pub parallel_execution_enabled: bool,
    pub parallel_execution_max_in_flight: usize,
    pub observed_peak_in_flight: usize,
    pub observed_wall_time_ms: u64,
    pub segments: Vec<ToolBatchExecutionSegmentTrace>,
}

impl ToolBatchExecutionSegmentTrace {
    fn record_observation(&mut self, observed_peak_in_flight: usize, observed_wall_time_ms: u64) {
        self.observed_peak_in_flight = Some(observed_peak_in_flight);
        self.observed_wall_time_ms = Some(observed_wall_time_ms);
    }
}

impl ToolBatchExecutionTrace {
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
    intent: ToolIntent,
    request: ToolCoreRequest,
    execution_kind: ToolExecutionKind,
    scheduling_class: ToolSchedulingClass,
    trusted_internal_context: bool,
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
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return Err(TurnFailure::policy_denied("tool_not_found", reason));
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
                if descriptor.name == "tool.invoke" {
                    // Lease validation happens in resolve_tool_invoke_request during execution.
                } else if !crate::tools::is_provider_exposed_tool_name(&intent.tool_name) {
                    let reason = format!("tool_not_provider_exposed: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied(
                        "tool_not_provider_exposed",
                        reason,
                    ));
                }
            } else {
                if !session_context
                    .tool_view
                    .contains(resolved_tool.canonical_name)
                {
                    let reason = format!("tool_not_visible: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }
                if intent.source.starts_with("provider_") {
                    return Err(concealed_provider_tool_denial());
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
    ) -> (TurnResult, Option<ToolBatchExecutionTrace>) {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => return (TurnResult::FinalText(text), None),
            Err(failure) => return (TurnResult::ToolDenied(failure), None),
            Ok(TurnValidation::ToolExecutionRequired) => {}
        }

        let mut prepared = Vec::new();
        for intent in &turn.tool_intents {
            match self
                .prepare_tool_intent(intent, session_context, app_dispatcher, binding, ingress)
                .await
            {
                Ok(prepared_intent) => prepared.push(prepared_intent),
                Err(result) => return (result, None),
            }
        }
        let batch_segments = self.prepared_batch_segments(&prepared);
        let mut trace = self.trace_prepared_batch(&prepared, &batch_segments);

        let outputs = match self
            .execute_prepared_batch(
                &prepared,
                &batch_segments,
                session_context,
                app_dispatcher,
                binding,
                &mut trace,
            )
            .await
        {
            Ok(outputs) => outputs,
            Err(result) => return (result, Some(trace)),
        };

        (TurnResult::FinalText(outputs.join("\n")), Some(trace))
    }

    async fn execute_prepared_batch<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared: &[PreparedToolIntent],
        batch_segments: &[PreparedBatchSegment],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        trace: &mut ToolBatchExecutionTrace,
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
                            trace_segment,
                        )
                        .await?
                    }
                    ToolBatchExecutionMode::Sequential => {
                        self.execute_prepared_batch_sequential(
                            prepared_segment,
                            session_context,
                            app_dispatcher,
                            binding,
                            trace_segment,
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

    fn trace_prepared_batch(
        &self,
        prepared: &[PreparedToolIntent],
        batch_segments: &[PreparedBatchSegment],
    ) -> ToolBatchExecutionTrace {
        ToolBatchExecutionTrace {
            total_intents: prepared.len(),
            parallel_execution_enabled: self.parallel_tool_execution_enabled,
            parallel_execution_max_in_flight: self.parallel_tool_execution_max_in_flight,
            observed_peak_in_flight: 0,
            observed_wall_time_ms: 0,
            segments: batch_segments
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
                .collect(),
        }
    }

    fn prepared_batch_segments(
        &self,
        prepared: &[PreparedToolIntent],
    ) -> Vec<PreparedBatchSegment> {
        let mut segments = Vec::new();
        let mut remaining = prepared;
        while let Some((first, _)) = remaining.split_first() {
            let scheduling_class = first.scheduling_class;
            let len = remaining
                .iter()
                .take_while(|prepared_intent| prepared_intent.scheduling_class == scheduling_class)
                .count();
            segments.push(PreparedBatchSegment {
                len,
                scheduling_class,
                execution_mode: self.segment_execution_mode(scheduling_class, len),
            });
            let (_, rest) = remaining.split_at(len);
            remaining = rest;
        }
        segments
    }

    fn segment_execution_mode(
        &self,
        scheduling_class: ToolSchedulingClass,
        segment_len: usize,
    ) -> ToolBatchExecutionMode {
        if self.parallel_tool_execution_enabled
            && self.parallel_tool_execution_max_in_flight > 1
            && scheduling_class == ToolSchedulingClass::ParallelSafe
            && segment_len > 1
        {
            ToolBatchExecutionMode::Parallel
        } else {
            ToolBatchExecutionMode::Sequential
        }
    }

    async fn execute_prepared_batch_sequential<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let result = async {
            let mut outputs = Vec::with_capacity(prepared.len());
            for prepared_intent in prepared {
                let outcome = self
                    .execute_prepared_tool_intent(
                        prepared_intent,
                        session_context,
                        app_dispatcher,
                        binding,
                    )
                    .await?;
                outputs.push(format_tool_result_line_with_limit(
                    &prepared_intent.intent,
                    &outcome,
                    self.tool_result_payload_summary_limit_chars,
                ));
            }
            Ok(outputs)
        }
        .await;
        trace_segment.record_observation(
            if prepared.is_empty() { 0 } else { 1 },
            elapsed_ms_u64(started_at),
        );
        result
    }

    async fn execute_prepared_batch_in_parallel<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared: &[PreparedToolIntent],
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        trace_segment: &mut ToolBatchExecutionSegmentTrace,
    ) -> Result<Vec<String>, TurnResult> {
        let started_at = Instant::now();
        let payload_summary_limit_chars = self.tool_result_payload_summary_limit_chars;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let observed_peak = Arc::new(AtomicUsize::new(0));
        let mut results = Vec::with_capacity(prepared.len());
        let mut executions = stream::iter(prepared.iter().cloned().enumerate().map(
            |(index, prepared_intent)| {
                let in_flight = Arc::clone(&in_flight);
                let observed_peak = Arc::clone(&observed_peak);
                async move {
                    let current_in_flight = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                    observe_peak_in_flight(observed_peak.as_ref(), current_in_flight);
                    let result = self
                        .execute_prepared_tool_intent(
                            &prepared_intent,
                            session_context,
                            app_dispatcher,
                            binding,
                        )
                        .await
                        .map(|outcome| {
                            format_tool_result_line_with_limit(
                                &prepared_intent.intent,
                                &outcome,
                                payload_summary_limit_chars,
                            )
                        });
                    in_flight.fetch_sub(1, Ordering::Relaxed);
                    (index, result)
                }
            },
        ))
        .buffer_unordered(self.parallel_tool_execution_max_in_flight);

        let result = async {
            while let Some((index, result)) = executions.next().await {
                match result {
                    Ok(output) => results.push((index, output)),
                    Err(turn_result) => return Err(turn_result),
                }
            }
            Ok(())
        }
        .await;
        trace_segment.record_observation(
            observed_peak.load(Ordering::Relaxed),
            elapsed_ms_u64(started_at),
        );
        result?;
        results.sort_by_key(|(index, _)| *index);

        Ok(results.into_iter().map(|(_, output)| output).collect())
    }

    async fn prepare_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        intent: &ToolIntent,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> Result<PreparedToolIntent, TurnResult> {
        let Some(resolved_tool) = crate::tools::resolve_tool_execution(&intent.tool_name) else {
            let reason = format!("tool_not_found: {}", intent.tool_name);
            return Err(TurnResult::policy_denied("tool_not_found", reason));
        };
        let injected = inject_internal_tool_ingress(
            resolved_tool.canonical_name,
            intent.args_json.clone(),
            ingress,
        );
        let augmented_payload = augment_tool_payload_for_kernel(
            resolved_tool.canonical_name,
            injected.payload,
            session_context,
        );
        let request = ToolCoreRequest {
            tool_name: resolved_tool.canonical_name.to_owned(),
            payload: augmented_payload,
        };
        let (effective_execution_kind, effective_request, effective_intent) =
            if resolved_tool.canonical_name == "tool.invoke" {
                match crate::tools::resolve_tool_invoke_request(&request) {
                    Ok((inner_resolved, inner_request))
                        if inner_resolved.execution_kind == ToolExecutionKind::App =>
                    {
                        let inner_intent = ToolIntent {
                            tool_name: inner_resolved.canonical_name.to_owned(),
                            args_json: inner_request.payload.clone(),
                            source: intent.source.clone(),
                            session_id: intent.session_id.clone(),
                            turn_id: intent.turn_id.clone(),
                            tool_call_id: intent.tool_call_id.clone(),
                        };
                        (ToolExecutionKind::App, inner_request, inner_intent)
                    }
                    _ => (resolved_tool.execution_kind, request, intent.clone()),
                }
            } else {
                (resolved_tool.execution_kind, request, intent.clone())
            };
        let catalog = crate::tools::tool_catalog();
        let Some(descriptor) = catalog.resolve(effective_request.tool_name.as_str()) else {
            let reason = format!("tool_descriptor_missing: {}", effective_request.tool_name);
            return Err(TurnResult::non_retryable_tool_error(
                "tool_descriptor_missing",
                reason,
            ));
        };
        let scheduling_class = descriptor.scheduling_class();

        match effective_execution_kind {
            ToolExecutionKind::Core => {
                if binding.kernel_context().is_none() {
                    return Err(TurnResult::policy_denied(
                        "no_kernel_context",
                        "no_kernel_context",
                    ));
                }
            }
            ToolExecutionKind::App => {
                match app_dispatcher
                    .maybe_require_approval_with_binding(
                        session_context,
                        &effective_intent,
                        descriptor,
                        binding,
                    )
                    .await
                {
                    Ok(Some(requirement)) => return Err(TurnResult::NeedsApproval(requirement)),
                    Ok(None) => {}
                    Err(reason) if reason.starts_with("app_tool_denied:") => {
                        return Err(TurnResult::policy_denied("app_tool_denied", reason));
                    }
                    Err(reason) => {
                        return Err(TurnResult::non_retryable_tool_error(
                            "app_tool_preflight_failed",
                            reason,
                        ));
                    }
                }
            }
        }

        Ok(PreparedToolIntent {
            intent: intent.clone(),
            request: effective_request,
            execution_kind: effective_execution_kind,
            scheduling_class,
            trusted_internal_context: injected.trusted_internal_context,
        })
    }

    async fn execute_prepared_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared_intent: &PreparedToolIntent,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolCoreOutcome, TurnResult> {
        match prepared_intent.execution_kind {
            ToolExecutionKind::Core => {
                let Some(kernel_ctx) = binding.kernel_context() else {
                    return Err(TurnResult::policy_denied(
                        "no_kernel_context",
                        "no_kernel_context",
                    ));
                };
                execute_tool_intent_via_kernel(
                    prepared_intent.request.clone(),
                    kernel_ctx,
                    prepared_intent.trusted_internal_context,
                )
                .await
                .map_err(turn_result_from_tool_execution_failure)
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
                    Err(TurnResult::policy_denied("tool_not_found", reason))
                }
                Err(reason) if reason.starts_with("app_tool_disabled:") => {
                    Err(TurnResult::policy_denied("app_tool_disabled", reason))
                }
                Err(reason) if reason.starts_with("app_tool_denied:") => {
                    Err(TurnResult::policy_denied("app_tool_denied", reason))
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
    use crate::test_support::unique_temp_dir;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

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
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '1.2.3\\n'\n  exit 0\nfi\nBODY=\"$(cat)\"\nprintf '%s' \"$BODY\" > \"{}\"\nprintf '%s' '{}'\n",
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
                ConversationRuntimeBinding::direct(),
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
    }

    #[tokio::test]
    async fn governed_tool_approval_request_is_persisted_for_discovered_delegate_async() {
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

        let mut tool_config = ToolConfig::default();
        tool_config.approval.mode = GovernedToolApprovalMode::Strict;
        let tool_view = runtime_tool_view_for_config(&tool_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), tool_config);

        let result = TurnEngine::new(4)
            .execute_turn_in_context(
                &discovered_delegate_async_turn(
                    "root-session",
                    "turn-discovered",
                    "call-discovered",
                ),
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
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
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
                None,
            )
            .await;
        let second = TurnEngine::new(4)
            .execute_turn_in_context(
                &turn,
                &session_context,
                &dispatcher,
                ConversationRuntimeBinding::direct(),
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
                ConversationRuntimeBinding::direct(),
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
    async fn browser_companion_click_turn_executes_when_approval_is_disabled() {
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

        let root = unique_browser_companion_temp_dir("loongclaw-turn-engine-browser-companion");
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
            loongclaw_contracts::ToolCoreRequest {
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

        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "true");

        let mut tool_config = ToolConfig::default();
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some(script_path.display().to_string());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, tool_config);

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
                ConversationRuntimeBinding::direct(),
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
            reply.contains("\"tool\":\"browser.companion.click\""),
            "reply should include the executed companion tool: {reply}"
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
    async fn browser_companion_click_turn_uses_runtime_visible_readiness_without_env_recheck() {
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

        let root =
            unique_browser_companion_temp_dir("loongclaw-turn-engine-browser-companion-runtime");
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
            loongclaw_contracts::ToolCoreRequest {
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

        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "false");

        let mut tool_config = ToolConfig::default();
        tool_config.browser_companion.enabled = true;
        tool_config.browser_companion.command = Some(script_path.display().to_string());

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, tool_config);

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
                ConversationRuntimeBinding::direct(),
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
            reply.contains("\"tool\":\"browser.companion.click\""),
            "reply should include the executed companion tool: {reply}"
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
    async fn browser_companion_click_turn_uses_runtime_visible_policy_when_app_config_is_default() {
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

        let root = unique_browser_companion_temp_dir(
            "loongclaw-turn-engine-browser-companion-runtime-policy",
        );
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
            loongclaw_contracts::ToolCoreRequest {
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

        let mut env = crate::test_support::ScopedEnv::new();
        env.set("LOONGCLAW_BROWSER_COMPANION_ENABLED", "true");
        env.set("LOONGCLAW_BROWSER_COMPANION_READY", "false");
        env.set(
            "LOONGCLAW_BROWSER_COMPANION_COMMAND",
            script_path.display().to_string(),
        );

        let tool_view = crate::tools::runtime_tool_view_for_runtime_config(&runtime_config);
        let session_context = SessionContext::root_with_tool_view("root-session", tool_view);
        let dispatcher = DefaultAppToolDispatcher::new(memory_config, ToolConfig::default());

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
                ConversationRuntimeBinding::direct(),
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
            reply.contains("\"tool\":\"browser.companion.click\""),
            "reply should include the executed companion tool: {reply}"
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
        let augmented = augment_tool_payload_for_kernel(&tool_name, payload, &session_context);

        assert_eq!(augmented["tool_id"], "browser.companion.session.start");
        assert_eq!(
            augmented["arguments"][crate::tools::BROWSER_SESSION_SCOPE_FIELD],
            "root-session"
        );
    }
}
