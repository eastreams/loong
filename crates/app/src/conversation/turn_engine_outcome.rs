use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

use super::ToolDecisionTelemetry;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFailure {
    pub kind: TurnFailureKind,
    pub code: String,
    pub reason: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "turn_failure_flag_is_false")]
    pub supports_discovery_recovery: bool,
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
