use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use loongclaw_contracts::{
    Capability, KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError,
};
use serde::{Deserialize, Serialize};

use crate::config::{LoongClawConfig, SessionVisibility, ToolConfig};
use crate::context::KernelContext;
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{SessionKind, SessionRepository};
use crate::tools::{
    ToolExecutionKind, ToolView, delegate_child_tool_view_for_config,
    delegate_child_tool_view_for_config_with_delegate, runtime_tool_view,
    runtime_tool_view_for_config, tool_catalog,
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
            TurnResult::FinalText(_) => None,
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
        KernelError::ToolPlane(ToolPlaneError::Execution(_)) => {
            KernelFailureClass::RetryableExecution
        }
        _ => KernelFailureClass::NonRetryable,
    }
}

#[async_trait]
pub trait AppToolDispatcher: Send + Sync {
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
}

impl Default for DefaultAppToolDispatcher {
    fn default() -> Self {
        Self::runtime()
    }
}

#[async_trait]
impl AppToolDispatcher for DefaultAppToolDispatcher {
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

pub(crate) async fn execute_tool_intent_via_kernel(
    intent: &ToolIntent,
    kernel_ctx: &KernelContext,
) -> Result<ToolCoreOutcome, TurnFailure> {
    let request = ToolCoreRequest {
        tool_name: crate::tools::canonical_tool_name(intent.tool_name.as_str()).to_owned(),
        payload: intent.args_json.clone(),
    };
    let caps = BTreeSet::from([Capability::InvokeTool]);
    kernel_ctx
        .kernel
        .execute_tool_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|error| {
            let reason = format!("{error}");
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
            let Some(descriptor) = catalog.resolve(&intent.tool_name) else {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return Err(TurnFailure::policy_denied("tool_not_found", reason));
            };
            if !session_context.tool_view.contains(descriptor.name) {
                let reason = format!("tool_not_visible: {}", intent.tool_name);
                return Err(TurnFailure::policy_denied("tool_not_visible", reason));
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
        self.execute_turn_in_view(turn, &runtime_tool_view(), Some(kernel_ctx))
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
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => return TurnResult::FinalText(text),
            Err(failure) => return TurnResult::ToolDenied(failure),
            Ok(TurnValidation::ToolExecutionRequired) => {}
        }

        let catalog = tool_catalog();
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
                    let Some(kernel_ctx) = kernel_ctx else {
                        return TurnResult::policy_denied("no_kernel_context", "no_kernel_context");
                    };
                    match execute_tool_intent_via_kernel(intent, kernel_ctx).await {
                        Ok(outcome) => outcome,
                        Err(failure) => return turn_result_from_tool_execution_failure(failure),
                    }
                }
                ToolExecutionKind::App => match app_dispatcher
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
                    Err(reason) => {
                        return TurnResult::non_retryable_tool_error(
                            "app_tool_execution_failed",
                            reason,
                        );
                    }
                },
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
