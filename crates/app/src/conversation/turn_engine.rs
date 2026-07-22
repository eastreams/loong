use loong_contracts::ToolPath;
use loong_contracts::{KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{GovernedToolApprovalMode, SessionVisibility, ToolConfig, ToolConsentMode};
use crate::context::Context;
#[cfg(feature = "memory-sqlite")]
use crate::operator::approval_runtime::OperatorApprovalRuntime;
#[cfg(feature = "memory-sqlite")]
use crate::operator::session_graph::OperatorSessionGraph;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    NewApprovalRequestRecord, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};
#[cfg(test)]
use crate::session::store;
use crate::session::store::SessionStoreConfig;
#[cfg(all(feature = "memory-sqlite", test))]
use crate::task_progress::TASK_PROGRESS_EVENT_KIND;
use crate::tools::{ToolApprovalMode, ToolOwner, governance_profile_for_descriptor, tool_catalog};
#[cfg(feature = "memory-sqlite")]
use crate::trust::{approval_required_trust_event, embed_trust_event_payload};

use super::autonomy_policy::{
    AUTONOMY_POLICY_SOURCE, AutonomyTurnBudgetState, PolicyDecision, PolicyDecisionInput,
    evaluate_policy, render_reason,
};
use super::turn_observer::{ConversationTurnObserverHandle, ConversationTurnRuntimeEvent};

use super::ingress::ConversationIngressContext;
use super::tool_input_contract::detect_repairable_tool_request_issue;

#[path = "turn_engine_batch.rs"]
mod batch;
#[path = "turn_engine_decision.rs"]
mod decision;
#[path = "turn_engine_dispatch_default.rs"]
mod dispatch_default;
#[path = "turn_engine_dispatcher.rs"]
mod dispatcher;
#[path = "turn_engine_execute.rs"]
mod execute;
#[path = "turn_engine_outcome.rs"]
mod outcome;
#[path = "turn_engine_payload.rs"]
mod payload;
#[path = "turn_engine_prepare.rs"]
mod prepare;
#[path = "turn_engine_result.rs"]
mod result;
#[path = "turn_engine_support.rs"]
mod support;
#[path = "turn_engine_trace.rs"]
mod trace;
#[path = "turn_engine_validate.rs"]
mod validate;
#[path = "turn_engine_visibility.rs"]
mod visibility;
use batch::ToolBatchHarness;
pub(crate) use decision::ToolOutcomeTelemetry;
pub use decision::{ToolDecision, ToolDecisionKind, ToolDecisionTelemetry, ToolOutcome};
pub use dispatcher::DefaultLegacyToolDispatcher;
use dispatcher::LegacyGovernedToolPreflight;
pub(crate) use dispatcher::LegacyToolDispatcher;
#[cfg(test)]
pub(crate) use dispatcher::NoopLegacyToolDispatcher;
pub(crate) use dispatcher::{LegacyToolDispatchKind, LegacyToolExecutionPreflight};
pub use outcome::{
    ApprovalRequirement, ApprovalRequirementKind, ToolInputFailure, ToolResultEnvelope,
    ToolResultPayloadSemantics, TurnFailure, TurnFailureKind, TurnResult, TurnValidation,
};
pub(crate) use outcome::{
    KernelFailureClass, LegacyToolPreflightOutcome, PreparedToolExecutionOutcome,
};
#[cfg(test)]
use payload::augment_tool_payload_for_kernel;
pub(crate) use payload::render_kernel_error_reason;
pub(crate) use result::{
    build_denied_tool_outcome_trace_record, build_failure_tool_outcome_trace_record,
    build_success_tool_outcome_trace_record, build_tool_decision_trace_record,
    build_tool_intent_completed_trace, build_tool_intent_denied_trace,
    build_tool_intent_failure_trace, effective_denied_tool_name, effective_result_tool_name,
    format_tool_denied_result_line_with_limit, format_tool_result_line_with_limit,
};
pub(crate) use support::classify_kernel_error;
use support::{
    LegacyRepairablePreflight, approval_required_tool_decision, denied_tool_decision,
    generic_allow_tool_decision, render_app_tool_denied_reason,
};
pub(crate) use trace::{
    ToolBatchExecutionIntentStatus, ToolBatchExecutionIntentTrace, ToolBatchExecutionMode,
    ToolBatchExecutionSegmentTrace, ToolBatchExecutionTrace, ToolDecisionTraceRecord,
    ToolOutcomeTraceRecord, elapsed_ms_u64, observe_peak_in_flight,
};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub assistant_text: String,
    pub tool_intents: Vec<ToolIntent>,
    pub raw_meta: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "resolution", rename_all = "snake_case")]
pub enum ToolIntentTarget {
    /// Exact runtime registration selected by the request-scoped provider surface.
    Registered {
        path: ToolPath,
        provider_name: String,
    },
    /// Raw ingress name whose owner has not yet been selected.
    ///
    /// Only this branch may attempt typed lookup and then enter the explicit
    /// legacy fallback after `NotRegistered`.
    Unresolved { name: String },
}

impl ToolIntentTarget {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Registered { provider_name, .. } => provider_name,
            Self::Unresolved { name } => name,
        }
    }

    #[must_use]
    pub(crate) fn registered_path(&self) -> Option<&ToolPath> {
        match self {
            Self::Registered { path, .. } => Some(path),
            Self::Unresolved { .. } => None,
        }
    }

    #[must_use]
    pub(crate) fn registered(path: ToolPath, provider_name: impl Into<String>) -> Self {
        Self::Registered {
            path,
            provider_name: provider_name.into(),
        }
    }
}

impl std::fmt::Display for ToolIntentTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl From<String> for ToolIntentTarget {
    fn from(name: String) -> Self {
        Self::Unresolved { name }
    }
}

impl From<&str> for ToolIntentTarget {
    fn from(name: &str) -> Self {
        Self::Unresolved {
            name: name.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntent {
    pub tool_name: ToolIntentTarget,
    pub args_json: serde_json::Value,
    pub source: String,
    pub turn_id: String,
    pub tool_call_id: String,
}

impl ToolIntent {
    #[must_use]
    pub fn tool_name(&self) -> &str {
        self.tool_name.name()
    }
}

struct AugmentedToolPayload {
    payload: serde_json::Value,
    trusted_internal_context: bool,
}

const TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 2048;
const MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 256;
const MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 64_000;
const TOOL_PREFLIGHT_ALLOW_RULE_ID: &str = "tool_preflight_allowed";
const AUTONOMY_POLICY_ALLOW_RULE_ID: &str = "autonomy_policy_allow";
const AUTONOMY_POLICY_ALLOW_REASON_CODE: &str = "autonomy_policy_allow";

fn governed_approval_request_id(
    session_context: &Context<'_>,
    tool_name: &str,
    intent: &ToolIntent,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(session_context.session().session_id.as_bytes());
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

/// Keep approval-control replay inside its explicit legacy owner.
///
/// These synthesized intents were not emitted by a provider surface, so the
/// provider-exposure check does not apply; all other legacy intents must pass it.
fn tool_intent_skips_provider_exposed_gate(
    intent: &ToolIntent,
    descriptor: &crate::tools::ToolDescriptor,
) -> bool {
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

/// Single orchestration boundary for tool-call evaluation and execution.
///
/// `evaluate_turn` performs synchronous validation (no execution).
/// `execute_turn` performs policy-gated tool execution through the kernel.
pub struct TurnEngine {
    tool_result_payload_summary_limit_chars: usize,
    parallel_tool_execution_enabled: bool,
    parallel_tool_execution_max_in_flight: usize,
}

impl TurnEngine {
    pub fn new(_max_tool_steps: usize) -> Self {
        Self::with_parallel_tool_execution(0, TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS, false, 1)
    }

    pub fn with_tool_result_payload_summary_limit(
        _max_tool_steps: usize,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self::with_parallel_tool_execution(0, tool_result_payload_summary_limit_chars, false, 1)
    }

    pub fn with_parallel_tool_execution(
        _max_tool_steps: usize,
        tool_result_payload_summary_limit_chars: usize,
        parallel_tool_execution_enabled: bool,
        parallel_tool_execution_max_in_flight: usize,
    ) -> Self {
        Self {
            tool_result_payload_summary_limit_chars: tool_result_payload_summary_limit_chars.clamp(
                MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
                MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
            ),
            parallel_tool_execution_enabled,
            parallel_tool_execution_max_in_flight: parallel_tool_execution_max_in_flight.max(1),
        }
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn approval_control_consent_exempt_tools_still_skip_provider_exposed_gate() {
        let descriptor = crate::tools::tool_catalog()
            .resolve("approval_request_status")
            .expect("approval_request_status descriptor should exist");
        let intent = ToolIntent {
            tool_name: "approval_request_status".into(),
            args_json: json!({}),
            source: "approval_control".to_owned(),
            turn_id: "turn".to_owned(),
            tool_call_id: "call".to_owned(),
        };

        assert!(tool_intent_skips_provider_exposed_gate(&intent, descriptor));
    }
}

#[cfg(test)]
#[path = "turn_engine_tests.rs"]
mod tests;
