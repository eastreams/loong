use std::fmt;
use std::ops::Deref;

use loong_contracts::ToolInputError;
use loong_contracts::ToolPath;
use serde::{Deserialize, Serialize};

use super::ToolDecisionTelemetry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirementKind {
    ContextRequired,
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
pub(crate) enum LegacyToolPreflightOutcome {
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
    SkillContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnFailureKind {
    PolicyDenied,
    Retryable,
    NonRetryable,
    Provider,
}

/// Structured typed-tool input rejection retained across orchestration layers.
///
/// Runtime path is execution identity, while provider name is presentation.
/// Keeping both prevents follow-up repair from consulting the legacy catalog or
/// reconstructing identity from an error string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInputFailure {
    pub path: ToolPath,
    pub provider_name: String,
    pub argument_hint: Option<String>,
    pub error: ToolInputError,
}

impl ToolInputFailure {
    /// Render model-facing repair instructions from typed input evidence only.
    #[must_use]
    pub fn repair_guidance(&self) -> String {
        let mut lines = vec![format!("Repair guidance for {}:", self.provider_name)];
        match &self.error {
            ToolInputError::PayloadMustBeObject => {
                lines.push("Send a JSON object payload instead of a scalar or list.".to_owned());
            }
            ToolInputError::MissingOneOf { fields } => {
                let fields = fields
                    .iter()
                    .map(|field| format!("`{field}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("Add at least one of these fields: {fields}."));
            }
            ToolInputError::MissingField { field } => {
                lines.push(format!("Add required field `payload.{field}`."));
            }
            ToolInputError::InvalidField { field, reason } => {
                lines.push(format!(
                    "Fix `payload.{field}`: {}.",
                    reason.trim_end_matches('.')
                ));
            }
            ToolInputError::InvalidPayload { reason } => {
                lines.push(format!(
                    "Fix the payload: {}.",
                    reason.trim_end_matches('.')
                ));
            }
            error => {
                lines.push(format!("Fix the payload: {error}."));
            }
        }
        if let Some(argument_hint) = self
            .argument_hint
            .as_deref()
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
        {
            lines.push(format!("Expected payload shape: {argument_hint}."));
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFailure {
    pub kind: TurnFailureKind,
    pub code: String,
    pub reason: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "turn_failure_flag_is_false")]
    pub supports_discovery_recovery: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<Box<ToolInputFailure>>,
}

fn turn_failure_flag_is_false(value: &bool) -> bool {
    !*value
}

impl TurnFailure {
    pub fn policy_denied(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::PolicyDenied,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
            tool_input: None,
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
            tool_input: None,
        }
    }

    pub fn retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Retryable,
            code: code.into(),
            reason: reason.into(),
            retryable: true,
            supports_discovery_recovery: false,
            tool_input: None,
        }
    }

    pub fn input_repair_required(reason: impl Into<String>, tool_input: ToolInputFailure) -> Self {
        Self {
            // Re-executing the same payload cannot recover. Structured input
            // detail drives a separate model-repair path after execution stops.
            kind: TurnFailureKind::NonRetryable,
            code: "tool_input_invalid".to_owned(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
            tool_input: Some(Box::new(tool_input)),
        }
    }

    pub fn non_retryable(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::NonRetryable,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
            tool_input: None,
        }
    }

    pub fn provider(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            kind: TurnFailureKind::Provider,
            code: code.into(),
            reason: reason.into(),
            retryable: false,
            supports_discovery_recovery: false,
            tool_input: None,
        }
    }

    pub fn as_str(&self) -> &str {
        self.reason.as_str()
    }

    pub(crate) fn into_turn_result(self) -> TurnResult {
        match self.kind {
            TurnFailureKind::PolicyDenied => TurnResult::ToolDenied(self),
            TurnFailureKind::Retryable | TurnFailureKind::NonRetryable => {
                TurnResult::ToolError(self)
            }
            TurnFailureKind::Provider => TurnResult::ProviderError(self),
        }
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

#[derive(Debug, Clone)]
pub(crate) enum PreparedToolExecutionOutcome {
    Completed {
        status: String,
        payload: serde_json::Value,
    },
    Denied(TurnFailure),
    Interrupted(TurnResult),
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
