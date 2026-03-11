use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;

use loongclaw_contracts::{Capability, KernelError, ToolCoreRequest, ToolPlaneError};
use serde::{Deserialize, Serialize};

use crate::context::KernelContext;

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
    pub approval_required: bool,
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
pub enum TurnFailureKind {
    ApprovalRequired,
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
    pub fn approval_required(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::ApprovalRequired,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
        }
    }

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
    NeedsApproval(TurnFailure),
    ToolDenied(TurnFailure),
    ToolError(TurnFailure),
    ProviderError(TurnFailure),
}

impl TurnResult {
    pub fn needs_approval(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::NeedsApproval(TurnFailure::approval_required(code, reason))
    }

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
            TurnResult::NeedsApproval(failure)
            | TurnResult::ToolDenied(failure)
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

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    max_tool_steps: usize,
}

impl TurnEngine {
    pub fn new(max_tool_steps: usize) -> Self {
        Self { max_tool_steps }
    }

    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        // No tool intents → just return the text
        if turn.tool_intents.is_empty() {
            return TurnResult::FinalText(turn.assistant_text.clone());
        }

        // Too many tool intents for current step limit
        if turn.tool_intents.len() > self.max_tool_steps {
            return TurnResult::policy_denied("max_tool_steps_exceeded", "max_tool_steps_exceeded");
        }

        // Check each tool intent
        for intent in &turn.tool_intents {
            if !crate::tools::is_known_tool_name(&intent.tool_name) {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_found", reason);
            }
        }

        // All tools validated — execution requires a kernel context
        TurnResult::needs_approval("kernel_context_required", "kernel_context_required")
    }

    /// Execute a provider turn with policy-gated tool execution through the kernel.
    ///
    /// Flow:
    /// 1. No tool intents → `FinalText`
    /// 2. Too many intents → `ToolDenied("max_tool_steps_exceeded")`
    /// 3. Unknown tool → `ToolDenied("tool_not_found: ...")`
    /// 4. No kernel context → `ToolDenied("no_kernel_context")`
    /// 5. Policy/capability check via kernel → `ToolDenied` with reason if denied
    /// 6. Execute tool → map result to `TurnResult`
    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
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
        for intent in &turn.tool_intents {
            if !crate::tools::is_known_tool_name(&intent.tool_name) {
                let reason = format!("tool_not_found: {}", intent.tool_name);
                return TurnResult::policy_denied("tool_not_found", reason);
            }
        }

        // Require kernel context for execution
        let ctx = match kernel_ctx {
            Some(ctx) => ctx,
            None => return TurnResult::policy_denied("no_kernel_context", "no_kernel_context"),
        };

        // Execute each tool intent through the kernel
        let mut outputs = Vec::new();
        for intent in &turn.tool_intents {
            let request = ToolCoreRequest {
                tool_name: intent.tool_name.clone(),
                payload: intent.args_json.clone(),
            };
            let caps = BTreeSet::from([Capability::InvokeTool]);
            match ctx
                .kernel
                .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
            {
                Ok(outcome) => {
                    outputs.push(format!("[{}] {}", outcome.status, outcome.payload));
                }
                Err(e) => {
                    let reason = format!("{e}");
                    return match classify_kernel_error(&e) {
                        KernelFailureClass::PolicyDenied => {
                            TurnResult::policy_denied("kernel_policy_denied", reason)
                        }
                        KernelFailureClass::RetryableExecution => {
                            TurnResult::retryable_tool_error("tool_execution_failed", reason)
                        }
                        KernelFailureClass::NonRetryable => {
                            TurnResult::non_retryable_tool_error("kernel_execution_failed", reason)
                        }
                    };
                }
            }
        }

        TurnResult::FinalText(outputs.join("\n"))
    }
}
