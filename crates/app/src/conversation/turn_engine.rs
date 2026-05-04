use loong_contracts::{KernelError, ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{GovernedToolApprovalMode, SessionVisibility, ToolConfig, ToolConsentMode};
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
use crate::tools::{
    ToolApprovalMode, ToolDescriptor, ToolExecutionKind, ToolView,
    delegate_child_tool_view_for_contract, governance_profile_for_descriptor, runtime_tool_view,
    runtime_tool_view_for_config, tool_catalog,
};
#[cfg(feature = "memory-sqlite")]
use crate::trust::{approval_required_trust_event, embed_trust_event_payload};

use super::autonomy_policy::{
    AUTONOMY_POLICY_SOURCE, AutonomyTurnBudgetState, PolicyDecision, PolicyDecisionInput,
    evaluate_policy, render_reason,
};
use super::runtime::{SessionContext, load_default_conversation_runtime};
use super::runtime_binding::ConversationRuntimeBinding;
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
#[path = "turn_engine_target.rs"]
mod target;
#[path = "turn_engine_trace.rs"]
mod trace;
#[path = "turn_engine_validate.rs"]
mod validate;
#[path = "turn_engine_visibility.rs"]
mod visibility;
use batch::ToolBatchHarness;
pub(crate) use decision::ToolOutcomeTelemetry;
pub use decision::{ToolDecision, ToolDecisionKind, ToolDecisionTelemetry, ToolOutcome};
use dispatcher::GovernedToolPreflight;
pub use dispatcher::{
    AppToolDispatcher, DefaultAppToolDispatcher, NoopAppToolDispatcher, ToolExecutionPreflight,
};
use execute::session_context_from_turn;
pub(crate) use outcome::KernelFailureClass;
pub use outcome::{
    ApprovalRequirement, ApprovalRequirementKind, ToolPreflightOutcome, ToolResultEnvelope,
    ToolResultPayloadSemantics, TurnFailure, TurnFailureKind, TurnResult, TurnValidation,
};
#[cfg(test)]
use payload::augment_tool_payload_for_kernel;
pub(crate) use payload::render_kernel_error_reason;
use prepare::{PreparedToolIntent, PreparedToolIntentFailure, ToolIntentPreparationHarness};
pub(crate) use result::{
    build_failure_tool_outcome_trace_record, build_success_tool_outcome_trace_record,
    build_tool_decision_trace_record, build_tool_intent_completed_trace,
    build_tool_intent_failure_trace, effective_denied_tool_name, effective_result_tool_name,
    format_tool_result_line_with_limit, turn_result_from_tool_execution_failure,
};
pub(crate) use support::classify_kernel_error;
use support::{
    RepairableToolPreflight, approval_required_tool_decision, denied_tool_decision,
    generic_allow_tool_decision, render_app_tool_denied_reason,
};
pub(crate) use trace::{
    ToolBatchExecutionIntentStatus, ToolBatchExecutionIntentTrace, ToolBatchExecutionMode,
    ToolBatchExecutionSegmentTrace, ToolBatchExecutionTrace, ToolDecisionTraceRecord,
    ToolOutcomeTraceRecord, elapsed_ms_u64, observe_peak_in_flight,
};
use visibility::{
    concealed_provider_tool_denial, effective_visible_tool_name, provider_tool_denial_reason,
    provider_tool_denial_should_conceal_name, tool_intent_is_visible,
};

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

const TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 2048;
const MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 256;
const MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS: usize = 64_000;
const TOOL_PREFLIGHT_ALLOW_RULE_ID: &str = "tool_preflight_allowed";
const AUTONOMY_POLICY_ALLOW_RULE_ID: &str = "autonomy_policy_allow";
const AUTONOMY_POLICY_ALLOW_REASON_CODE: &str = "autonomy_policy_allow";

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
    fn tool_invoke_no_longer_skips_provider_exposed_gate() {
        let descriptor = crate::tools::tool_catalog()
            .resolve("file.read")
            .expect("file.read descriptor should exist");
        let intent = ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: json!({}),
            source: "provider_tool_call".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn".to_owned(),
            tool_call_id: "call".to_owned(),
        };

        assert!(!tool_intent_skips_provider_exposed_gate(
            &intent, descriptor
        ));
    }

    #[test]
    fn approval_control_consent_exempt_tools_still_skip_provider_exposed_gate() {
        let descriptor = crate::tools::tool_catalog()
            .resolve("approval_request_status")
            .expect("approval_request_status descriptor should exist");
        let intent = ToolIntent {
            tool_name: "approval_request_status".to_owned(),
            args_json: json!({}),
            source: "approval_control".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn".to_owned(),
            tool_call_id: "call".to_owned(),
        };

        assert!(tool_intent_skips_provider_exposed_gate(&intent, descriptor));
    }
}

#[cfg(test)]
#[path = "turn_engine_tests.rs"]
mod tests;
