#[cfg(test)]
use super::super::config::LoongConfig;
#[cfg(test)]
use super::runtime::ConversationRuntime;
#[cfg(test)]
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_engine::{
    ApprovalRequirement, ApprovalRequirementKind, ToolResultPayloadSemantics, TurnResult,
};
#[cfg(test)]
use super::turn_engine::{ProviderTurn, ToolIntent, ToolResultEnvelope};
use serde::Serialize;
#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
use crate::CliResult;
#[cfg(test)]
use crate::tools::ToolView;

#[path = "turn_shared_approval.rs"]
mod approval;
#[path = "turn_shared_control.rs"]
mod control;
#[path = "turn_shared_external_skill.rs"]
mod external_skill;
#[path = "turn_shared_followup_tail.rs"]
mod followup_tail;
#[path = "turn_shared_payload.rs"]
mod payload;
#[path = "turn_shared_prompt.rs"]
mod prompt;
#[path = "turn_shared_reply.rs"]
mod reply;
#[path = "turn_shared_request.rs"]
mod request;
#[path = "turn_shared_runtime.rs"]
mod runtime_support;
#[path = "turn_shared_tool_result.rs"]
mod tool_result;
pub use approval::{
    ApprovalPromptActionId, ApprovalPromptActionView, ApprovalPromptLocale, ApprovalPromptMarker,
    ApprovalPromptView, format_approval_required_reply, normalize_approval_prompt_control_input,
    parse_approval_prompt_action_input, parse_approval_prompt_view,
};
pub(crate) use control::salvage_missing_tool_call_reply_text;
pub(crate) use control::sanitize_reply_text;
#[cfg(test)]
pub(crate) use control::{MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS, strip_think_tags};
pub use control::{
    ParsedToolDrivenContinuationReply, ToolDrivenContinuationState,
    missing_tool_call_followup_payload, next_conversation_turn_id,
    parse_tool_driven_continuation_reply, tool_loop_circuit_breaker_reply,
};
pub(crate) use control::{
    ToolDrivenFollowupContractMode, render_tool_followup_continuation_contract,
};
pub use external_skill::{
    ExternalSkillInvokeContext, external_skill_invoke_context_from_payload_summary,
    parse_external_skill_invoke_context,
};
pub(crate) use followup_tail::build_tool_driven_followup_tail_with_request_summary_and_contract;
pub use followup_tail::build_tool_loop_guard_tail;
#[cfg(test)]
pub use followup_tail::{
    build_tool_driven_followup_tail, build_tool_failure_followup_tail,
    build_tool_result_followup_tail,
};
#[cfg(test)]
pub use payload::ToolDrivenFollowupMessageOwned;
#[cfg(test)]
pub use payload::turn_failure_supports_discovery_recovery;
pub use payload::{
    ToolDrivenFollowupKind, ToolDrivenFollowupLabel, ToolDrivenFollowupPayload,
    ToolDrivenFollowupTextRef, ToolResultLine, tool_driven_followup_payload,
};
#[cfg(test)]
pub use prompt::build_tool_followup_user_prompt;
pub use prompt::{
    EXTERNAL_SKILL_FOLLOWUP_PROMPT, TOOL_LOOP_GUARD_PROMPT, TOOL_TRUNCATION_HINT_PROMPT,
    build_discovery_recovery_followup_user_prompt, build_tool_followup_user_prompt_with_context,
    join_non_empty_lines,
};
pub(crate) use prompt::{
    append_followup_preface, append_followup_warning, combine_followup_extra_context,
};
pub use reply::{
    ToolDrivenReplyBaseDecision, ToolDrivenReplyPhase, user_requested_raw_tool_output,
};
#[cfg(test)]
pub use reply::{ToolDrivenReplyKernel, compose_assistant_reply};
#[cfg(test)]
pub(crate) use request::summarize_failed_provider_lane_tool_request;
pub(crate) use request::{
    effective_followup_visible_tool_name, summarize_provider_lane_tool_request,
    summarize_single_tool_followup_request,
};
pub use runtime_support::{
    ProviderTurnRequestAction, decide_provider_turn_request_action,
    request_completion_with_raw_fallback, request_completion_with_raw_fallback_detailed,
};
pub(crate) use tool_result::ToolResultContinuation;
pub(crate) use tool_result::{parse_tool_result_continuation, parse_tool_result_followup_context};
pub use tool_result::{reduce_followup_payload_for_model, tool_result_contains_truncation_signal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyPersistenceMode {
    Success,
    InlineProviderError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyResolutionMode {
    Direct,
    CompletionPass,
}

#[cfg(test)]
pub(crate) fn parse_tool_result_followup_for_test(messages: &[Value]) -> (Value, Value) {
    let assistant_tool_result = messages
        .iter()
        .find(|message| {
            message.get("role") == Some(&Value::String("assistant".to_owned()))
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
        })
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .and_then(ToolDrivenFollowupMessageOwned::parse_assistant_content)
        .expect("assistant tool_result followup message should exist");
    assert_eq!(
        assistant_tool_result.label(),
        ToolDrivenFollowupLabel::ToolResult
    );
    let tool_result_line = ToolResultLine::parse(assistant_tool_result.body())
        .expect("tool result line should preserve structured envelope");
    let envelope = serde_json::to_value(tool_result_line.envelope())
        .expect("tool result envelope should serialize");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("payload summary should stay valid json");
    (envelope, summary)
}

#[cfg(test)]
#[path = "turn_shared_tests.rs"]
mod tests;
