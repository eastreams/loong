use super::*;
use crate::conversation::{
    FAST_LANE_PARALLEL_TOOL_EXECUTION_ENABLED, FAST_LANE_PARALLEL_TOOL_EXECUTION_MAX_IN_FLIGHT,
    TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
};

pub(super) async fn execute_provider_turn_lane<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &Context<'_>,
    preparation: &ProviderTurnPreparation,
    turn: &ProviderTurn,
    legacy_tools: &DefaultLegacyToolDispatcher,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    followup_chain_active: bool,
) -> ProviderTurnLaneExecution {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let provider_originated_tool_intents = turn
        .tool_intents
        .iter()
        .any(|intent| intent.source.starts_with("provider_"));
    let search_tool_intents = 0usize;
    let discovery_search_turn = false;
    let assistant_preface = turn.assistant_text.clone();
    let textual_tool_parse_followup_signal = provider_turn_has_textual_tool_parse_followup_signal(
        &turn.raw_meta,
        had_tool_intents,
        assistant_preface.as_str(),
    );
    let lane = preparation.lane_plan.decision.lane;
    let legacy_dispatcher = CoordinatorLegacyToolDispatcher {
        config,
        runtime,
        fallback: legacy_tools,
    };
    let payload_summary_limit_chars = TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS;
    let parallel_tool_execution_enabled =
        matches!(lane, ExecutionLane::Fast) && FAST_LANE_PARALLEL_TOOL_EXECUTION_ENABLED;
    let parallel_tool_execution_max_in_flight = if parallel_tool_execution_enabled {
        FAST_LANE_PARALLEL_TOOL_EXECUTION_MAX_IN_FLIGHT
    } else {
        1
    };
    let use_safe_lane_plan_path = preparation
        .lane_plan
        .should_use_safe_lane_plan_path(config, turn);
    let engine = TurnEngine::with_parallel_tool_execution(
        0,
        payload_summary_limit_chars,
        parallel_tool_execution_enabled,
        parallel_tool_execution_max_in_flight,
    );
    let validation = if use_safe_lane_plan_path {
        TurnEngine::with_tool_result_payload_summary_limit(usize::MAX, payload_summary_limit_chars)
            .classify_turn(turn)
    } else {
        engine.classify_turn(turn)
    };
    let (turn_result, safe_lane_terminal_route, fast_lane_tool_batch_trace) = match validation {
        TurnValidation::FinalText(text) => (TurnResult::FinalText(text), None, None),
        TurnValidation::ToolExecutionRequired if use_safe_lane_plan_path => {
            let outcome = execute_turn_with_safe_lane_plan(
                config,
                runtime,
                &preparation.lane_plan.decision,
                turn,
                session_context,
                &legacy_dispatcher,
                ingress,
            )
            .await;
            (outcome.result, outcome.terminal_route, None)
        }
        TurnValidation::ToolExecutionRequired => {
            let (result, trace) = engine
                .execute_turn_in_context_with_trace(
                    turn,
                    session_context,
                    &legacy_dispatcher,
                    ingress,
                    observer,
                )
                .await;
            (result, None, trace)
        }
    };

    if let Some(trace) = fast_lane_tool_batch_trace.as_ref() {
        if let Err(reason) = persist_fast_lane_tool_trace(runtime, trace, session_context).await {
            let _ = session_context.runtime().record_audit_event(
                Some(session_context.agent_id()),
                AuditEventKind::RuntimeOperation {
                    operation: "conversation.fast_lane.tool_trace_persist_failed".to_owned(),
                    outcome: RuntimeOperationOutcome::Failed { reason },
                },
            );
        }

        let should_emit_batch_event = trace.has_execution_segments();
        if should_emit_batch_event
            && let Err(reason) =
                emit_fast_lane_tool_batch_event(runtime, trace, session_context).await
        {
            let _ = session_context.runtime().record_audit_event(
                Some(session_context.agent_id()),
                AuditEventKind::RuntimeOperation {
                    operation: "conversation.fast_lane.fast_lane_tool_batch_persist_failed"
                        .to_owned(),
                    outcome: RuntimeOperationOutcome::Failed { reason },
                },
            );
        }
    }

    let tool_events = build_provider_turn_tool_terminal_events(
        turn,
        &turn_result,
        fast_lane_tool_batch_trace.as_ref(),
    );
    let tool_request_summary = summarize_provider_lane_tool_request(
        turn,
        &turn_result,
        fast_lane_tool_batch_trace.as_ref(),
    );
    let recovery_followup_turn = tool_driven_followup_payload(had_tool_intents, &turn_result)
        .is_some_and(|payload| {
            matches!(payload, ToolDrivenFollowupPayload::DiscoveryRecovery { .. })
        });
    let malformed_parse_followup_turn =
        provider_turn_has_malformed_parse_followup_signal(&turn.raw_meta);
    let runtime_followup_turn = tool_driven_followup_payload(had_tool_intents, &turn_result)
        .is_some_and(|payload| payload.requests_runtime_followup_chain());
    let supports_provider_turn_followup = followup_chain_active
        || discovery_search_turn
        || recovery_followup_turn
        || malformed_parse_followup_turn
        || runtime_followup_turn
        || textual_tool_parse_followup_signal;
    ProviderTurnLaneExecution {
        lane,
        assistant_preface,
        provider_usage: provider_turn_usage(turn),
        had_tool_intents,
        provider_originated_tool_intents,
        textual_tool_parse_followup_turn: textual_tool_parse_followup_signal,
        tool_request_summary,
        discovery_search_turn,
        search_tool_intents,
        malformed_parse_followup_turn,
        supports_provider_turn_followup,
        raw_tool_output_requested: preparation.raw_tool_output_requested,
        turn_result,
        safe_lane_terminal_route,
        tool_events,
    }
}

pub(super) fn provider_turn_has_malformed_parse_followup_signal(raw_meta: &Value) -> bool {
    let Some(parse_meta) = raw_meta.get("loong_provider_parse") else {
        return false;
    };
    let Some(parse_meta_object) = parse_meta.as_object() else {
        return false;
    };

    parse_meta_object.values().any(|entry| {
        let status = entry.get("status").and_then(Value::as_str);
        status == Some("malformed")
    })
}

fn provider_turn_has_textual_tool_parse_followup_signal(
    raw_meta: &Value,
    had_tool_intents: bool,
    assistant_preface: &str,
) -> bool {
    if !had_tool_intents || assistant_preface.trim().is_empty() {
        return false;
    }

    let Some(parse_meta) = raw_meta.get("loong_provider_parse") else {
        return false;
    };
    let Some(parse_meta_object) = parse_meta.as_object() else {
        return false;
    };

    parse_meta_object.values().any(|entry| {
        let status = entry.get("status").and_then(Value::as_str);
        status == Some("parsed")
    })
}
