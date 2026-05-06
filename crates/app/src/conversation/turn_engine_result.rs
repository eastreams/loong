use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::json;

use super::super::tool_result_compaction::compact_discovery_payload_summary;
use super::super::turn_shared::effective_followup_visible_tool_name;
use super::{
    MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS, MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS, ToolBatchExecutionIntentStatus,
    ToolBatchExecutionIntentTrace, ToolDecisionTelemetry, ToolDecisionTraceRecord, ToolIntent,
    ToolOutcomeTelemetry, ToolOutcomeTraceRecord, ToolResultEnvelope, ToolResultPayloadSemantics,
    TurnFailure, TurnFailureKind, TurnResult,
};

pub(crate) fn turn_result_from_tool_execution_failure(failure: TurnFailure) -> TurnResult {
    match failure.kind {
        TurnFailureKind::PolicyDenied => TurnResult::ToolDenied(failure),
        TurnFailureKind::Retryable | TurnFailureKind::NonRetryable => {
            TurnResult::ToolError(failure)
        }
        TurnFailureKind::Provider => TurnResult::ProviderError(failure),
    }
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

pub(crate) fn build_tool_result_envelope(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
    payload_summary_limit_chars: usize,
) -> ToolResultEnvelope {
    let effective_tool_name = effective_result_tool_name(intent);
    let payload_semantics = detect_tool_result_payload_semantics(&outcome.payload);
    let normalized_limit = payload_summary_limit_chars.clamp(
        MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    );
    let compacted_payload = compact_tool_result_payload_value(payload_semantics, &outcome.payload);
    let payload_text = serde_json::to_string(&compacted_payload)
        .unwrap_or_else(|_| "[tool_payload_unserializable]".to_owned());
    let (payload_summary, payload_chars, payload_truncated) =
        summarize_tool_result_payload(payload_text.as_str(), payload_semantics, normalized_limit);

    ToolResultEnvelope {
        status: outcome.status.clone(),
        tool: effective_tool_name,
        tool_call_id: intent.tool_call_id.clone(),
        payload_semantics,
        payload_summary,
        payload_chars,
        payload_truncated,
    }
}

fn compact_tool_result_payload_value(
    payload_semantics: Option<ToolResultPayloadSemantics>,
    payload: &serde_json::Value,
) -> serde_json::Value {
    if let Some(compacted_payload) = compact_continuation_payload_summary(payload) {
        return compacted_payload;
    }

    if payload_semantics == Some(ToolResultPayloadSemantics::DiscoveryResult)
        && let Some(compacted_payload) = compact_discovery_payload_summary(payload)
    {
        return compacted_payload;
    }

    payload.clone()
}

fn compact_continuation_payload_summary(payload: &serde_json::Value) -> Option<serde_json::Value> {
    let payload_object = payload.as_object()?;
    let continuation_object = payload_object.get("continuation")?.as_object()?;

    let mut compacted = serde_json::Map::new();
    for key in [
        "mode",
        "profile",
        "label",
        "state",
        "wait_status",
        "task_id",
    ] {
        if let Some(value) = payload_object.get(key) {
            compacted.insert(key.to_owned(), value.clone());
        }
    }

    let mut compacted_continuation = serde_json::Map::new();
    for key in [
        "state",
        "is_terminal",
        "recommended_tool",
        "recommended_payload",
    ] {
        if let Some(value) = continuation_object.get(key) {
            compacted_continuation.insert(key.to_owned(), value.clone());
        }
    }
    compacted.insert(
        "continuation".to_owned(),
        serde_json::Value::Object(compacted_continuation),
    );
    Some(serde_json::Value::Object(compacted))
}

fn summarize_tool_result_payload(
    payload_text: &str,
    payload_semantics: Option<ToolResultPayloadSemantics>,
    payload_summary_limit_chars: usize,
) -> (String, usize, bool) {
    if payload_semantics.is_some() {
        let payload_chars = payload_text.chars().count();
        return (payload_text.to_owned(), payload_chars, false);
    }

    truncate_by_chars(payload_text, payload_summary_limit_chars)
}

fn detect_tool_result_payload_semantics(
    payload: &serde_json::Value,
) -> Option<ToolResultPayloadSemantics> {
    if payload_looks_like_discovery_result(payload) {
        return Some(ToolResultPayloadSemantics::DiscoveryResult);
    }
    if payload_looks_like_skill_context(payload) {
        return Some(ToolResultPayloadSemantics::SkillContext);
    }
    None
}

fn payload_looks_like_discovery_result(payload: &serde_json::Value) -> bool {
    let Some(payload_object) = payload.as_object() else {
        return false;
    };
    let Some(results) = payload_object
        .get("results")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };

    if results.is_empty() {
        return payload_object.contains_key("query");
    }

    results.iter().any(|result| {
        let Some(result_object) = result.as_object() else {
            return false;
        };
        result_object
            .get("tool_id")
            .and_then(serde_json::Value::as_str)
            .is_some()
            && result_object
                .get("lease")
                .and_then(serde_json::Value::as_str)
                .is_some()
    })
}

fn payload_looks_like_skill_context(payload: &serde_json::Value) -> bool {
    let Some(payload_object) = payload.as_object() else {
        return false;
    };
    payload_object
        .get("skill_id")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && payload_object
            .get("display_name")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && payload_object
            .get("instructions")
            .and_then(serde_json::Value::as_str)
            .is_some()
}

pub(crate) fn effective_result_tool_name(intent: &ToolIntent) -> String {
    let request = ToolCoreRequest {
        tool_name: intent.tool_name.clone(),
        payload: intent.args_json.clone(),
    };
    let canonical_tool_name = crate::tools::peek_tool_invoke_request(&request)
        .map(|peeked| peeked.tool_name)
        .unwrap_or_else(|| crate::tools::canonical_tool_name(intent.tool_name.as_str()));
    crate::tools::user_visible_tool_name(canonical_tool_name)
}

pub(crate) fn effective_denied_tool_name(intent: &ToolIntent) -> String {
    effective_followup_visible_tool_name(intent)
}

pub(crate) fn build_tool_decision_trace_record(
    intent: &ToolIntent,
    decision: ToolDecisionTelemetry,
) -> ToolDecisionTraceRecord {
    ToolDecisionTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        decision,
    }
}

pub(crate) fn build_success_tool_outcome_trace_record(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> ToolOutcomeTraceRecord {
    let tool_name = effective_result_tool_name(intent);
    let outcome = ToolOutcomeTelemetry {
        tool_name,
        status: outcome.status.clone(),
        payload: build_bounded_tool_outcome_payload(intent, outcome),
        error_code: None,
        human_reason: None,
        audit_event_id: None,
    };
    ToolOutcomeTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        outcome,
    }
}

fn build_bounded_tool_outcome_payload(
    _intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> serde_json::Value {
    let payload_semantics = detect_tool_result_payload_semantics(&outcome.payload);
    let normalized_limit = TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS.clamp(
        MIN_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        MAX_TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    );
    let payload_text = serde_json::to_string(&outcome.payload)
        .unwrap_or_else(|_| "[tool_payload_unserializable]".to_owned());
    let (payload_summary, payload_chars, payload_truncated) =
        summarize_tool_result_payload(payload_text.as_str(), payload_semantics, normalized_limit);

    if !payload_truncated {
        return outcome.payload.clone();
    }

    json!({
        "payload_summary": payload_summary,
        "payload_chars": payload_chars,
        "payload_truncated": true,
    })
}

pub(crate) fn build_failure_tool_outcome_trace_record(
    intent: &ToolIntent,
    turn_result: &TurnResult,
) -> Option<ToolOutcomeTraceRecord> {
    let failure = turn_result.failure()?;
    let tool_name = effective_result_tool_name(intent);
    let outcome = ToolOutcomeTelemetry {
        tool_name,
        status: "error".to_owned(),
        payload: serde_json::Value::Null,
        error_code: Some(failure.code.clone()),
        human_reason: Some(failure.reason.clone()),
        audit_event_id: None,
    };
    Some(ToolOutcomeTraceRecord {
        turn_id: intent.turn_id.clone(),
        tool_call_id: intent.tool_call_id.clone(),
        outcome,
    })
}

pub(crate) fn build_tool_intent_completed_trace(
    intent: &ToolIntent,
    outcome: &ToolCoreOutcome,
) -> ToolBatchExecutionIntentTrace {
    let tool_name = effective_result_tool_name(intent);
    let detail = summarize_completed_tool_trace_detail(tool_name.as_str(), outcome);

    ToolBatchExecutionIntentTrace {
        tool_call_id: intent.tool_call_id.clone(),
        tool_name,
        status: ToolBatchExecutionIntentStatus::Completed,
        detail,
    }
}

fn summarize_completed_tool_trace_detail(
    tool_name: &str,
    outcome: &ToolCoreOutcome,
) -> Option<String> {
    let normalized_status = outcome.status.trim();
    if !normalized_status.is_empty() && normalized_status != "ok" {
        return Some(normalized_status.to_owned());
    }

    let payload_semantics = detect_tool_result_payload_semantics(&outcome.payload);
    if payload_semantics == Some(ToolResultPayloadSemantics::DiscoveryResult) {
        return summarize_tool_search_completed_trace_detail(&outcome.payload);
    }

    let _ = tool_name;
    None
}

fn summarize_tool_search_completed_trace_detail(payload: &serde_json::Value) -> Option<String> {
    let returned = payload.get("returned")?.as_u64()?;
    let noun = if returned == 1 { "result" } else { "results" };
    Some(format!("returned {returned} {noun}"))
}

pub(crate) fn build_tool_intent_failure_trace(
    intent: &ToolIntent,
    turn_result: &TurnResult,
) -> Option<ToolBatchExecutionIntentTrace> {
    let tool_name = effective_result_tool_name(intent);

    match turn_result {
        TurnResult::NeedsApproval(requirement) => Some(ToolBatchExecutionIntentTrace {
            tool_call_id: intent.tool_call_id.clone(),
            tool_name,
            status: ToolBatchExecutionIntentStatus::NeedsApproval,
            detail: Some(requirement.reason.clone()),
        }),
        TurnResult::ToolDenied(failure) => Some(ToolBatchExecutionIntentTrace {
            tool_call_id: intent.tool_call_id.clone(),
            tool_name,
            status: ToolBatchExecutionIntentStatus::Denied,
            detail: Some(failure.reason.clone()),
        }),
        TurnResult::ToolError(failure) | TurnResult::ProviderError(failure) => {
            Some(ToolBatchExecutionIntentTrace {
                tool_call_id: intent.tool_call_id.clone(),
                tool_name,
                status: ToolBatchExecutionIntentStatus::Failed,
                detail: Some(failure.reason.clone()),
            })
        }
        TurnResult::FinalText(_) | TurnResult::StreamingText(_) | TurnResult::StreamingDone(_) => {
            None
        }
    }
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
