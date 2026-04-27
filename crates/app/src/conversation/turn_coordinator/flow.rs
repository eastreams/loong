use super::*;

pub(super) async fn resolve_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    result: CliResult<ProviderTurn>,
    error_mode: ProviderErrorMode,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> ResolvedProviderTurn {
    let turn_loop_policy = ProviderTurnLoopPolicy::from_config(config);
    let mut turn_loop_state = ProviderTurnLoopState::default();

    match decide_provider_turn_request_action(result, error_mode) {
        ProviderTurnRequestAction::Continue { turn } => {
            let turn =
                scope_provider_turn_tool_intents(turn, session_id, preparation.turn_id.as_str());
            if let Some(reply) =
                turn_loop_state.circuit_breaker_reply(&turn_loop_policy, turn.tool_intents.len())
            {
                return build_turn_loop_circuit_breaker_resolved_turn(
                    preparation,
                    user_input,
                    turn.tool_intents.len(),
                    reply,
                );
            }
            let continue_phase = prepare_provider_turn_continue_phase(
                config,
                runtime,
                session_id,
                preparation,
                turn,
                &turn_loop_policy,
                &mut turn_loop_state,
                binding,
                ingress,
                observer,
                1,
                false,
            )
            .await;
            continue_phase
                .resolve(
                    runtime,
                    session_id,
                    preparation,
                    user_input,
                    &turn_loop_policy,
                    &mut turn_loop_state,
                    crate::conversation::TURN_LOOP_MAX_DISCOVERY_FOLLOWUP_ROUNDS
                        .saturating_add(1)
                        .max(1),
                    binding,
                    observer,
                    retry_progress,
                )
                .await
        }
        ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
            ProviderTurnRequestTerminalPhase::persist_inline_provider_error(reply)
                .resolve(preparation, user_input)
        }
        ProviderTurnRequestAction::ReturnError { error } => {
            ProviderTurnRequestTerminalPhase::return_error(error).resolve(preparation, user_input)
        }
    }
}

pub(super) fn scope_provider_turn_tool_intents(
    mut turn: ProviderTurn,
    session_id: &str,
    turn_id: &str,
) -> ProviderTurn {
    for intent in &mut turn.tool_intents {
        if intent.source.starts_with("provider_") {
            intent.session_id = session_id.to_owned();
            intent.turn_id = turn_id.to_owned();
        } else {
            if intent.session_id.trim().is_empty() {
                intent.session_id = session_id.to_owned();
            }
            if intent.turn_id.trim().is_empty() {
                intent.turn_id = turn_id.to_owned();
            }
        }
    }
    turn
}

pub(super) fn provider_turn_usage(turn: &ProviderTurn) -> Option<Value> {
    turn.raw_meta.get("usage").cloned()
}

pub(super) fn build_turn_loop_circuit_breaker_resolved_turn(
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    tool_intents: usize,
    reply: String,
) -> ResolvedProviderTurn {
    let checkpoint = build_resolved_provider_checkpoint(
        preparation,
        user_input,
        Some(reply.as_str()),
        TurnCheckpointRequest::Continue { tool_intents },
        None,
        None,
        TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
    );
    ResolvedProviderTurn::persist_reply(reply, None, checkpoint)
}

pub(super) async fn prepare_provider_turn_continue_phase<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    turn: ProviderTurn,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    provider_round: usize,
    followup_chain_active: bool,
) -> ProviderTurnContinuePhase {
    let tool_intents = turn.tool_intents.len();
    let lane = preparation.lane_plan.decision.lane;
    if tool_intents > 0 {
        let running_tools_event =
            ConversationTurnPhaseEvent::running_tools(provider_round, lane, tool_intents);
        observe_turn_phase(observer, running_tools_event);
        observe_provider_turn_tool_batch_started(observer, &turn);
    }
    let lane_execution = execute_provider_turn_lane(
        config,
        runtime,
        session_id,
        preparation,
        &turn,
        binding,
        ingress,
        observer,
        followup_chain_active,
    )
    .await;
    emit_runtime_binding_trust_event_if_needed(
        runtime,
        session_id,
        &lane_execution.turn_result,
        binding,
    )
    .await;
    observe_provider_turn_tool_batch_terminal(observer, &lane_execution.tool_events);
    let loop_verdict = turn_loop_state.observe_turn(turn_loop_policy, &turn);
    let followup_config =
        ConversationTurnCoordinator::reload_followup_provider_config_after_tool_turn(config, &turn);
    ProviderTurnContinuePhase::new(
        tool_intents,
        lane_execution,
        loop_verdict,
        followup_config,
        ingress,
    )
}
