use super::*;

pub(super) async fn execute_provider_turn_lane<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: &ProviderTurn,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    followup_chain_active: bool,
) -> ProviderTurnLaneExecution {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let search_tool_intents = turn
        .tool_intents
        .iter()
        .filter(|intent| effective_followup_tool_name(intent) == "tool.search")
        .count();
    let discovery_search_turn = search_tool_intents > 0;
    let malformed_parse_followup_turn =
        provider_turn_has_malformed_parse_followup_signal(&turn.raw_meta);
    let supports_provider_turn_followup =
        followup_chain_active || discovery_search_turn || malformed_parse_followup_turn;
    let assistant_preface = turn.assistant_text.clone();
    let lane = preparation.lane_plan.decision.lane;
    let session_context = match runtime.session_context(config, session_id, binding) {
        Ok(session_context) => session_context,
        Err(error) => {
            let turn_result = TurnResult::non_retryable_tool_error("session_context_failed", error);
            let tool_events = build_provider_turn_tool_terminal_events(turn, &turn_result, None);
            let tool_request_summary =
                summarize_provider_lane_tool_request(turn, &turn_result, None);
            return ProviderTurnLaneExecution {
                lane,
                assistant_preface,
                provider_usage: provider_turn_usage(turn),
                had_tool_intents,
                tool_request_summary,
                discovery_search_turn,
                search_tool_intents,
                malformed_parse_followup_turn,
                supports_provider_turn_followup,
                raw_tool_output_requested: preparation.raw_tool_output_requested,
                turn_result,
                safe_lane_terminal_route: None,
                tool_events,
            };
        }
    };
    let base_app_dispatcher = DefaultAppToolDispatcher::with_config(
        store::session_store_config_from_memory_config(&config.memory),
        config.clone(),
    );
    let app_dispatcher = CoordinatorAppToolDispatcher {
        config,
        runtime,
        fallback: &base_app_dispatcher,
    };
    let payload_summary_limit_chars = config
        .conversation
        .tool_result_payload_summary_limit_chars();
    let parallel_tool_execution_enabled = matches!(lane, ExecutionLane::Fast)
        && config
            .conversation
            .fast_lane_parallel_tool_execution_enabled;
    let parallel_tool_execution_max_in_flight = if parallel_tool_execution_enabled {
        config
            .conversation
            .fast_lane_parallel_tool_execution_max_in_flight()
    } else {
        1
    };
    let use_safe_lane_plan_path = preparation
        .lane_plan
        .should_use_safe_lane_plan_path(config, turn);
    let engine = TurnEngine::with_parallel_tool_execution(
        preparation.lane_plan.max_tool_steps,
        payload_summary_limit_chars,
        parallel_tool_execution_enabled,
        parallel_tool_execution_max_in_flight,
    );
    let validation = if use_safe_lane_plan_path {
        TurnEngine::with_tool_result_payload_summary_limit(usize::MAX, payload_summary_limit_chars)
            .validate_turn_in_context(turn, &session_context)
    } else {
        engine.validate_turn_in_context(turn, &session_context)
    };
    let (turn_result, safe_lane_terminal_route, fast_lane_tool_batch_trace) = match validation {
        Ok(TurnValidation::FinalText(text)) => (TurnResult::FinalText(text), None, None),
        Err(failure) => (TurnResult::ToolDenied(failure), None, None),
        Ok(TurnValidation::ToolExecutionRequired) if use_safe_lane_plan_path => {
            let outcome = execute_turn_with_safe_lane_plan(
                config,
                runtime,
                session_id,
                &preparation.lane_plan.decision,
                turn,
                &session_context,
                &app_dispatcher,
                binding,
                ingress,
            )
            .await;
            (outcome.result, outcome.terminal_route, None)
        }
        Ok(TurnValidation::ToolExecutionRequired) => {
            let (result, trace) = engine
                .execute_turn_in_context_with_trace(
                    turn,
                    &session_context,
                    &app_dispatcher,
                    binding,
                    ingress,
                    observer,
                )
                .await;
            (result, None, trace)
        }
    };

    if let Some(trace) = fast_lane_tool_batch_trace.as_ref() {
        let trace_persist_failed =
            persist_fast_lane_tool_trace(runtime, session_id, trace, binding)
                .await
                .is_err();
        if trace_persist_failed && let Some(ctx) = binding.kernel_context() {
            let _ = ctx.kernel.record_audit_event(
                Some(ctx.agent_id()),
                AuditEventKind::PlaneInvoked {
                    pack_id: ctx.pack_id().to_owned(),
                    plane: ExecutionPlane::Runtime,
                    tier: PlaneTier::Core,
                    primary_adapter: "conversation.fast_lane".to_owned(),
                    delegated_core_adapter: None,
                    operation: "conversation.fast_lane.tool_trace_persist_failed".to_owned(),
                    required_capabilities: Vec::new(),
                },
            );
        }

        let should_emit_batch_event = trace.has_execution_segments();
        let batch_event_failed = should_emit_batch_event
            && emit_fast_lane_tool_batch_event(runtime, session_id, trace, binding)
                .await
                .is_err();
        if batch_event_failed && let Some(ctx) = binding.kernel_context() {
            let _ = ctx.kernel.record_audit_event(
                Some(ctx.agent_id()),
                AuditEventKind::PlaneInvoked {
                    pack_id: ctx.pack_id().to_owned(),
                    plane: ExecutionPlane::Runtime,
                    tier: PlaneTier::Core,
                    primary_adapter: "conversation.fast_lane".to_owned(),
                    delegated_core_adapter: None,
                    operation: "conversation.fast_lane.fast_lane_tool_batch_persist_failed"
                        .to_owned(),
                    required_capabilities: Vec::new(),
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
    let preface_signals_provider_turn_followup =
        assistant_preface_signals_provider_turn_followup(assistant_preface.as_str());
    let supports_provider_turn_followup = followup_chain_active
        || discovery_search_turn
        || recovery_followup_turn
        || malformed_parse_followup_turn
        || runtime_followup_turn
        || preface_signals_provider_turn_followup;
    ProviderTurnLaneExecution {
        lane,
        assistant_preface,
        provider_usage: provider_turn_usage(turn),
        had_tool_intents,
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

pub(super) fn assistant_preface_signals_provider_turn_followup(assistant_preface: &str) -> bool {
    let normalized_preface = assistant_preface.to_ascii_lowercase();
    let contains_first = normalized_preface.contains("first");
    let contains_then = normalized_preface.contains("then");
    let contains_next = normalized_preface.contains("next");
    let contains_after_that = normalized_preface.contains("after that");
    let contains_afterwards = normalized_preface.contains("afterwards");

    contains_first || contains_then || contains_next || contains_after_that || contains_afterwards
}
