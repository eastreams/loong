use serde_json::Value;

use super::super::turn_engine::{
    ProviderTurn, ToolBatchExecutionIntentStatus, ToolBatchExecutionTrace, ToolIntent, TurnResult,
};

pub(crate) fn summarize_tool_followup_request(intents: &[ToolIntent]) -> Option<String> {
    match intents {
        [] => None,
        [intent] => summarize_single_tool_followup_request(intent),
        intents => serde_json::to_string(
            &intents
                .iter()
                .map(tool_followup_request_entry)
                .collect::<Vec<_>>(),
        )
        .ok(),
    }
}

pub(crate) fn summarize_single_tool_followup_request(intent: &ToolIntent) -> Option<String> {
    let entry = tool_followup_request_entry(intent);
    serde_json::to_string(&entry).ok()
}

pub(crate) fn summarize_provider_lane_tool_request(
    turn: &ProviderTurn,
    turn_result: &TurnResult,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Option<String> {
    match turn_result {
        TurnResult::FinalText(_) | TurnResult::StreamingText(_) | TurnResult::StreamingDone(_) => {
            summarize_tool_followup_request(&turn.tool_intents)
        }
        TurnResult::NeedsApproval(_)
        | TurnResult::ToolDenied(_)
        | TurnResult::ToolError(_)
        | TurnResult::ProviderError(_) => summarize_failed_provider_lane_tool_request(turn, trace),
    }
}

pub(crate) fn summarize_failed_provider_lane_tool_request(
    turn: &ProviderTurn,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Option<String> {
    let failed_tool_call_id = trace.and_then(first_failed_provider_lane_tool_call_id);
    if let Some(failed_tool_call_id) = failed_tool_call_id {
        let failed_intent = turn
            .tool_intents
            .iter()
            .find(|intent| intent.tool_call_id == failed_tool_call_id)?;
        return summarize_single_tool_followup_request(failed_intent);
    }

    match turn.tool_intents.as_slice() {
        [intent] => summarize_single_tool_followup_request(intent),
        [] => None,
        _ => summarize_tool_followup_request(&turn.tool_intents),
    }
}

fn first_failed_provider_lane_tool_call_id(trace: &ToolBatchExecutionTrace) -> Option<&str> {
    let failed_outcome = trace.intent_outcomes.iter().find(|intent_outcome| {
        !matches!(
            intent_outcome.status,
            ToolBatchExecutionIntentStatus::Completed
        )
    })?;
    Some(failed_outcome.tool_call_id.as_str())
}

fn tool_followup_request_entry(intent: &ToolIntent) -> Value {
    let canonical_tool_name = effective_followup_tool_name(intent);
    let visible_tool_name = effective_followup_visible_tool_name(intent);
    let request = effective_followup_request(intent);
    let request = sanitize_followup_request_summary(canonical_tool_name.as_str(), request);
    serde_json::json!({
        "name": visible_tool_name,
        "arguments": request,
    })
}

fn sanitize_followup_request_summary(tool_name: &str, request: Value) -> Value {
    crate::tools::summarize_tool_request_for_display(tool_name, request)
}

pub(crate) fn effective_followup_tool_name(intent: &ToolIntent) -> String {
    let request = loong_contracts::ToolCoreRequest {
        tool_name: intent.tool_name.clone(),
        payload: intent.args_json.clone(),
    };
    crate::tools::peek_tool_invoke_request(&request)
        .map(|peeked| peeked.tool_name.to_owned())
        .unwrap_or_else(|| crate::tools::canonical_tool_name(intent.tool_name.as_str()).to_owned())
}

pub(crate) fn effective_followup_visible_tool_name(intent: &ToolIntent) -> String {
    let canonical_tool_name = effective_followup_tool_name(intent);
    crate::tools::user_visible_tool_name(canonical_tool_name.as_str())
}

pub(crate) fn effective_followup_request(intent: &ToolIntent) -> Value {
    let request = loong_contracts::ToolCoreRequest {
        tool_name: intent.tool_name.clone(),
        payload: intent.args_json.clone(),
    };
    let (canonical_tool_name, payload) = crate::tools::peek_tool_invoke_request(&request)
        .map(|peeked| {
            let mut payload = peeked.arguments.clone();
            let grouped_agent_wrapper = request
                .payload
                .get("tool_id")
                .and_then(Value::as_str)
                .is_some_and(|tool_id| tool_id == "agent");
            if grouped_agent_wrapper && let Some(payload_object) = payload.as_object_mut() {
                payload_object.remove("operation");
            }
            (peeked.tool_name, payload)
        })
        .unwrap_or_else(|| {
            (
                crate::tools::canonical_tool_name(intent.tool_name.as_str()),
                intent.args_json.clone(),
            )
        });
    #[cfg(feature = "tool-shell")]
    {
        crate::tools::normalize_shell_payload_for_request(canonical_tool_name, payload)
    }
    #[cfg(not(feature = "tool-shell"))]
    {
        let _ = canonical_tool_name;
        payload
    }
}
