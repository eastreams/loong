use super::*;

pub(super) fn observe_turn_phase(
    observer: Option<&ConversationTurnObserverHandle>,
    event: ConversationTurnPhaseEvent,
) {
    let Some(observer) = observer else {
        return;
    };

    observer.on_phase(event);
}

pub(super) fn observe_non_provider_turn_terminal_success_phases(
    observer: Option<&ConversationTurnObserverHandle>,
) {
    let finalizing_event = ConversationTurnPhaseEvent {
        phase: ConversationTurnPhase::FinalizingReply,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
    };
    observe_turn_phase(observer, finalizing_event);

    let completed_event = ConversationTurnPhaseEvent {
        phase: ConversationTurnPhase::Completed,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
    };
    observe_turn_phase(observer, completed_event);
}

pub(super) fn observe_provider_turn_tool_batch_started(
    observer: Option<&ConversationTurnObserverHandle>,
    turn: &ProviderTurn,
) {
    let Some(observer) = observer else {
        return;
    };

    for intent in &turn.tool_intents {
        let tool_name = effective_result_tool_name(intent);
        let request_summary = summarize_single_tool_followup_request(intent);
        let event = ConversationTurnToolEvent::running(intent.tool_call_id.clone(), tool_name)
            .with_request_summary(request_summary);
        observer.on_tool(event);
    }
}

pub(super) fn observe_provider_turn_tool_batch_terminal(
    observer: Option<&ConversationTurnObserverHandle>,
    tool_events: &[ConversationTurnToolEvent],
) {
    let Some(observer) = observer else {
        return;
    };

    for tool_event in tool_events {
        observer.on_tool(tool_event.clone());
    }
}

pub(super) fn build_provider_turn_tool_terminal_events(
    turn: &ProviderTurn,
    turn_result: &TurnResult,
    trace: Option<&ToolBatchExecutionTrace>,
) -> Vec<ConversationTurnToolEvent> {
    let mut trace_events = BTreeMap::new();
    if let Some(trace) = trace {
        for intent_outcome in &trace.intent_outcomes {
            let event = match intent_outcome.status {
                ToolBatchExecutionIntentStatus::Completed => ConversationTurnToolEvent::completed(
                    intent_outcome.tool_call_id.clone(),
                    intent_outcome.tool_name.clone(),
                    intent_outcome.detail.clone(),
                ),
                ToolBatchExecutionIntentStatus::NeedsApproval => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::needs_approval(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
                ToolBatchExecutionIntentStatus::Denied => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::denied(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
                ToolBatchExecutionIntentStatus::Failed => {
                    let detail = intent_outcome.detail.clone().unwrap_or_default();
                    ConversationTurnToolEvent::failed(
                        intent_outcome.tool_call_id.clone(),
                        intent_outcome.tool_name.clone(),
                        detail,
                    )
                }
            };
            trace_events.insert(intent_outcome.tool_call_id.clone(), event);
        }
    }

    let mut events = Vec::new();
    let mut unresolved_failure_emitted = false;

    for intent in &turn.tool_intents {
        if let Some(event) = trace_events.remove(intent.tool_call_id.as_str()) {
            let request_summary = summarize_single_tool_followup_request(intent);
            let event = event.with_request_summary(request_summary);
            events.push(event);
            continue;
        }

        let tool_name = effective_result_tool_name(intent);
        let fallback_event = match turn_result {
            TurnResult::FinalText(_)
            | TurnResult::StreamingText(_)
            | TurnResult::StreamingDone(_) => Some(ConversationTurnToolEvent::completed(
                intent.tool_call_id.clone(),
                tool_name,
                None,
            )),
            TurnResult::NeedsApproval(requirement) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::needs_approval(
                        intent.tool_call_id.clone(),
                        tool_name,
                        requirement.reason.clone(),
                    ))
                }
            }
            TurnResult::ToolDenied(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::denied(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
            TurnResult::ToolError(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::failed(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
            TurnResult::ProviderError(failure) => {
                if unresolved_failure_emitted {
                    None
                } else {
                    unresolved_failure_emitted = true;
                    Some(ConversationTurnToolEvent::interrupted(
                        intent.tool_call_id.clone(),
                        tool_name,
                        failure.reason.clone(),
                    ))
                }
            }
        };

        if let Some(fallback_event) = fallback_event {
            let request_summary = summarize_single_tool_followup_request(intent);
            let fallback_event = fallback_event.with_request_summary(request_summary);
            events.push(fallback_event);
        }
    }

    events
}

#[cfg(test)]
pub(super) fn summarize_tool_event_request(intent: &ToolIntent) -> Option<String> {
    summarize_single_tool_followup_request(intent)
}

pub(super) fn provider_turn_observer_supports_streaming(
    config: &LoongConfig,
    observer: Option<&ConversationTurnObserverHandle>,
) -> bool {
    if observer.is_none() {
        return false;
    }

    crate::provider::supports_turn_streaming_events(config)
}

pub(super) async fn request_provider_turn_with_observer<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    tool_view: &crate::tools::ToolView,
    binding: ConversationRuntimeBinding<'_>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ProviderTurn> {
    if let Some(observer) = observer
        && provider_turn_observer_supports_streaming(config, Some(observer))
    {
        let on_token = build_observer_streaming_token_callback(observer);
        return runtime
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
            )
            .await;
    }

    runtime
        .request_turn(config, session_id, turn_id, messages, tool_view, binding)
        .await
}

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
                    config
                        .conversation
                        .turn_loop
                        .max_discovery_followup_rounds
                        .saturating_add(1)
                        .max(1),
                    binding,
                    observer,
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
            // Provider-originated intents: runtime scope is authoritative.
            intent.session_id = session_id.to_owned();
            intent.turn_id = turn_id.to_owned();
        } else {
            // Non-provider intents: only fill in if missing.
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
    ResolvedProviderTurn::persist_reply(reply, checkpoint)
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
    let should_emit_binding_trust_event =
        !matches!(lane, ExecutionLane::Safe) || config.conversation.safe_lane_emit_runtime_events;
    if should_emit_binding_trust_event {
        emit_runtime_binding_trust_event_if_needed(
            runtime,
            session_id,
            &lane_execution.turn_result,
            binding,
        )
        .await;
    }
    observe_provider_turn_tool_batch_terminal(observer, &lane_execution.tool_events);
    let loop_verdict =
        turn_loop_state.observe_turn(turn_loop_policy, &turn, &lane_execution.turn_result);
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

pub(super) async fn resolve_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    continue_phase: &ProviderTurnContinuePhase,
    user_input: &str,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    remaining_provider_rounds: usize,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> ResolvedProviderTurn {
    enum ReplyLoopDecision {
        FinalizeDirect(String),
        Followup {
            raw_reply: String,
            payload: ToolDrivenFollowupPayload,
            requires_completion_pass: bool,
            loop_warning_reason: Option<String>,
        },
        GuardFollowup {
            raw_reply: String,
            reason: String,
            latest_tool_payload: Option<ToolDrivenFollowupPayload>,
        },
    }

    let mut current_preparation = preparation.clone();
    let mut current_continue_phase = continue_phase.clone();
    let mut remaining_provider_rounds = remaining_provider_rounds.max(1);
    let mut provider_round_index = 0usize;

    loop {
        let current_provider_round = provider_round_index.saturating_add(1);
        if current_continue_phase.lane_execution.discovery_search_turn {
            emit_discovery_first_event(
                runtime,
                session_id,
                "discovery_first_search_round",
                json!({
                    "provider_round": current_provider_round,
                    "search_tool_calls": current_continue_phase
                        .lane_execution
                        .search_tool_intents,
                    "raw_tool_output_requested": current_continue_phase
                        .lane_execution
                        .raw_tool_output_requested,
                    "initial_estimated_tokens": estimate_tokens_for_messages(
                        current_preparation.session.estimated_tokens,
                        &current_preparation.session.messages,
                    ),
                }),
                binding,
            )
            .await;
        }

        let reply_decision = match current_continue_phase.reply_phase.decision() {
            ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => {
                let latest_tool_payload = tool_driven_followup_payload(
                    current_continue_phase.lane_execution.had_tool_intents,
                    &current_continue_phase.lane_execution.turn_result,
                );
                if let Some(reason) = current_continue_phase.hard_stop_reason() {
                    ReplyLoopDecision::GuardFollowup {
                        raw_reply: reply.clone(),
                        reason: reason.to_owned(),
                        latest_tool_payload,
                    }
                } else if current_continue_phase
                    .lane_execution
                    .supports_provider_turn_followup
                    && (!current_continue_phase
                        .lane_execution
                        .raw_tool_output_requested
                        || current_continue_phase.lane_execution.discovery_search_turn
                        || assistant_preface_signals_provider_turn_followup(
                            current_continue_phase
                                .lane_execution
                                .assistant_preface
                                .as_str(),
                        ))
                    && let Some(payload) = latest_tool_payload
                {
                    ReplyLoopDecision::Followup {
                        raw_reply: reply.clone(),
                        payload,
                        requires_completion_pass: false,
                        loop_warning_reason: current_continue_phase
                            .loop_warning_reason()
                            .map(ToOwned::to_owned),
                    }
                } else {
                    ReplyLoopDecision::FinalizeDirect(reply.clone())
                }
            }
            ToolDrivenReplyBaseDecision::RequireFollowup {
                raw_reply,
                payload: followup,
            } => {
                if let Some(reason) = current_continue_phase.hard_stop_reason() {
                    ReplyLoopDecision::GuardFollowup {
                        raw_reply: raw_reply.clone(),
                        reason: reason.to_owned(),
                        latest_tool_payload: Some(followup.clone()),
                    }
                } else {
                    ReplyLoopDecision::Followup {
                        raw_reply: raw_reply.clone(),
                        payload: followup.clone(),
                        requires_completion_pass: true,
                        loop_warning_reason: current_continue_phase
                            .loop_warning_reason()
                            .map(ToOwned::to_owned),
                    }
                }
            }
        };

        match reply_decision {
            ReplyLoopDecision::FinalizeDirect(reply) => {
                let checkpoint = current_continue_phase.checkpoint(preparation, user_input, &reply);
                return ResolvedProviderTurn::persist_reply(reply, checkpoint);
            }
            ReplyLoopDecision::Followup {
                raw_reply,
                payload: followup,
                requires_completion_pass,
                loop_warning_reason,
            } => {
                let follow_up_messages = build_turn_reply_followup_messages_with_warning(
                    &current_preparation.session.messages,
                    current_continue_phase
                        .lane_execution
                        .assistant_preface
                        .as_str(),
                    followup.clone(),
                    current_continue_phase
                        .lane_execution
                        .tool_request_summary
                        .as_deref(),
                    user_input,
                    loop_warning_reason.as_deref(),
                );
                if current_continue_phase
                    .lane_execution
                    .supports_provider_turn_followup
                    && remaining_provider_rounds > 1
                {
                    let next_provider_round = current_provider_round.saturating_add(1);
                    remaining_provider_rounds -= 1;
                    let initial_estimated_tokens = estimate_tokens_for_messages(
                        current_preparation.session.estimated_tokens,
                        &current_preparation.session.messages,
                    );
                    let followup_request_estimated_tokens = estimate_tokens(&follow_up_messages);
                    let followup_added_estimated_tokens = initial_estimated_tokens
                        .zip(followup_request_estimated_tokens)
                        .map(|(initial, followup)| followup.saturating_sub(initial));
                    let followup_preparation =
                        current_preparation.for_followup_messages(follow_up_messages.clone());
                    let followup_tool_view = match runtime.tool_view(
                        &current_continue_phase.followup_config,
                        session_id,
                        binding,
                    ) {
                        Ok(tool_view) => tool_view,
                        Err(_error) => {
                            let checkpoint = current_continue_phase.checkpoint(
                                preparation,
                                user_input,
                                raw_reply.as_str(),
                            );
                            return ResolvedProviderTurn::persist_reply(raw_reply, checkpoint);
                        }
                    };
                    let followup_message_count = followup_preparation.session.messages.len();
                    let followup_context_estimated_tokens =
                        followup_preparation.session.estimated_tokens;
                    let followup_request_event =
                        ConversationTurnPhaseEvent::requesting_followup_provider(
                            next_provider_round,
                            current_continue_phase.lane_execution.lane,
                            current_continue_phase.tool_intent_count(),
                            followup_message_count,
                            followup_context_estimated_tokens,
                        );
                    observe_turn_phase(observer, followup_request_event);
                    emit_prompt_frame_event(
                        runtime,
                        session_id,
                        next_provider_round,
                        "followup",
                        followup_preparation.session.prompt_frame_summary(),
                        binding,
                    )
                    .await;
                    if current_continue_phase.lane_execution.discovery_search_turn {
                        emit_discovery_first_event(
                            runtime,
                            session_id,
                            "discovery_first_followup_requested",
                            json!({
                                "provider_round": provider_round_index.saturating_add(1),
                                "raw_tool_output_requested": current_continue_phase
                                    .lane_execution
                                    .raw_tool_output_requested,
                                "initial_estimated_tokens": initial_estimated_tokens,
                                "followup_estimated_tokens": followup_request_estimated_tokens,
                                "followup_added_estimated_tokens": followup_added_estimated_tokens,
                            }),
                            binding,
                        )
                        .await;
                    }
                    match decide_provider_turn_request_action(
                        request_provider_turn_with_observer(
                            &current_continue_phase.followup_config,
                            runtime,
                            session_id,
                            followup_preparation.turn_id.as_str(),
                            &followup_preparation.session.messages,
                            &followup_tool_view,
                            binding,
                            observer,
                        )
                        .await,
                        ProviderErrorMode::Propagate,
                    ) {
                        ProviderTurnRequestAction::Continue { turn } => {
                            let turn = scope_provider_turn_tool_intents(
                                turn,
                                session_id,
                                followup_preparation.turn_id.as_str(),
                            );
                            let followup_result = summarize_discovery_first_followup_turn(&turn);
                            if current_continue_phase.lane_execution.discovery_search_turn {
                                emit_discovery_first_event(
                                    runtime,
                                    session_id,
                                    "discovery_first_followup_result",
                                    json!({
                                        "provider_round": provider_round_index.saturating_add(1),
                                        "outcome": followup_result.outcome,
                                        "followup_tool_name": followup_result.followup_tool_name,
                                        "followup_target_tool_id": followup_result.followup_target_tool_id,
                                        "resolved_to_tool_invoke": followup_result
                                            .resolved_to_tool_invoke,
                                        "raw_tool_output_requested": current_continue_phase
                                            .lane_execution
                                            .raw_tool_output_requested,
                                    }),
                                    binding,
                                )
                                .await;
                            }
                            if let Some(reply) = turn_loop_state
                                .circuit_breaker_reply(turn_loop_policy, turn.tool_intents.len())
                            {
                                return build_turn_loop_circuit_breaker_resolved_turn(
                                    preparation,
                                    user_input,
                                    turn.tool_intents.len(),
                                    reply,
                                );
                            }
                            current_continue_phase = prepare_provider_turn_continue_phase(
                                &current_continue_phase.followup_config,
                                runtime,
                                session_id,
                                &followup_preparation,
                                turn,
                                turn_loop_policy,
                                turn_loop_state,
                                binding,
                                ingress,
                                observer,
                                next_provider_round,
                                current_continue_phase
                                    .lane_execution
                                    .supports_provider_turn_followup,
                            )
                            .await;
                            current_preparation = followup_preparation;
                            provider_round_index = provider_round_index.saturating_add(1);
                            continue;
                        }
                        ProviderTurnRequestAction::FinalizeInlineProviderError {
                            reply: provider_error_text,
                        }
                        | ProviderTurnRequestAction::ReturnError {
                            error: provider_error_text,
                        } => {
                            if current_continue_phase.lane_execution.discovery_search_turn {
                                emit_discovery_first_event(
                                    runtime,
                                    session_id,
                                    "discovery_first_followup_result",
                                    json!({
                                        "provider_round": provider_round_index.saturating_add(1),
                                        "outcome": "provider_error",
                                        "followup_tool_name": Value::Null,
                                        "followup_target_tool_id": Value::Null,
                                        "resolved_to_tool_invoke": false,
                                        "raw_tool_output_requested": current_continue_phase
                                            .lane_execution
                                            .raw_tool_output_requested,
                                    }),
                                    binding,
                                )
                                .await;
                            }
                            emit_provider_failover_trust_event_if_needed(
                                config,
                                runtime,
                                session_id,
                                provider_error_text.as_str(),
                                binding,
                            )
                            .await;
                            if requires_completion_pass {
                                let reply = request_completion_with_raw_fallback(
                                    runtime,
                                    &current_continue_phase.followup_config,
                                    &follow_up_messages,
                                    binding,
                                    raw_reply.as_str(),
                                    retry_progress.clone(),
                                )
                                .await;
                                let checkpoint = current_continue_phase.checkpoint(
                                    preparation,
                                    user_input,
                                    reply.as_str(),
                                );
                                return ResolvedProviderTurn::persist_reply(
                                    reply,
                                    current_continue_phase.lane_execution.provider_usage.clone(),
                                    checkpoint,
                                );
                            }
                            let checkpoint = current_continue_phase.checkpoint(
                                preparation,
                                user_input,
                                raw_reply.as_str(),
                            );
                            return ResolvedProviderTurn::persist_reply(raw_reply, checkpoint);
                        }
                    }
                }
                if requires_completion_pass {
                    let reply = request_completion_with_raw_fallback(
                        runtime,
                        &current_continue_phase.followup_config,
                        &follow_up_messages,
                        binding,
                        raw_reply.as_str(),
                        None,
                    )
                    .await;
                    let checkpoint =
                        current_continue_phase.checkpoint(preparation, user_input, reply.as_str());
                    return ResolvedProviderTurn::persist_reply(reply, checkpoint);
                }

                let checkpoint =
                    current_continue_phase.checkpoint(preparation, user_input, raw_reply.as_str());
                return ResolvedProviderTurn::persist_reply(raw_reply, checkpoint);
            }
            ReplyLoopDecision::GuardFollowup {
                raw_reply,
                reason,
                latest_tool_payload,
            } => {
                let guard_messages = build_turn_reply_guard_messages(
                    &current_preparation.session.messages,
                    current_continue_phase
                        .lane_execution
                        .assistant_preface
                        .as_str(),
                    reason.as_str(),
                    latest_tool_payload.as_ref(),
                    user_input,
                );
                let reply = request_completion_with_raw_fallback(
                    runtime,
                    &current_continue_phase.followup_config,
                    &guard_messages,
                    binding,
                    raw_reply.as_str(),
                    None,
                )
                .await;
                let checkpoint =
                    current_continue_phase.checkpoint(preparation, user_input, reply.as_str());
                return ResolvedProviderTurn::persist_reply(reply, checkpoint);
            }
        }
    }
}

#[cfg(test)]
pub(super) fn build_turn_reply_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    user_input: &str,
) -> Vec<Value> {
    build_turn_reply_followup_messages_with_warning(
        base_messages,
        assistant_preface,
        followup,
        None,
        user_input,
        None,
    )
}

pub(super) fn build_turn_reply_followup_messages_with_warning(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    tool_request_summary: Option<&str>,
    user_input: &str,
    loop_warning_reason: Option<&str>,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    messages.extend(build_tool_driven_followup_tail_with_request_summary(
        assistant_preface,
        &followup,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        |label, text| reduce_followup_payload_for_model(label, text).into_owned(),
    ));
    messages
}

pub(super) fn build_turn_reply_guard_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    reason: &str,
    latest_tool_payload: Option<&ToolDrivenFollowupPayload>,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    messages.extend(build_tool_loop_guard_tail(
        assistant_preface,
        reason,
        user_input,
        latest_tool_payload.map(ToolDrivenFollowupPayload::message_context),
        |label, text| reduce_followup_payload_for_model(label.as_str(), text).into_owned(),
    ));
    messages
}
