use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingToolContinuationExpectation {
    RetryableFailure,
    RetryableFailureAfterRepair,
}

impl MissingToolContinuationExpectation {
    fn from_tool_failure(payload: &ToolDrivenFollowupPayload) -> Option<Self> {
        match payload {
            ToolDrivenFollowupPayload::ToolFailure { reason, retryable }
                if *retryable && reason.starts_with("missing_tool_call_followup:") =>
            {
                Some(Self::RetryableFailure)
            }
            ToolDrivenFollowupPayload::ToolFailure { .. }
            | ToolDrivenFollowupPayload::ToolResult { .. }
            | ToolDrivenFollowupPayload::DiscoveryRecovery { .. } => None,
        }
    }

    fn contract_mode(self) -> ToolDrivenFollowupContractMode {
        match self {
            Self::RetryableFailure => ToolDrivenFollowupContractMode::RetryableFailure,
            Self::RetryableFailureAfterRepair => {
                ToolDrivenFollowupContractMode::RepairRetryableFailure
            }
        }
    }

    fn after_attempt(self) -> Self {
        match self {
            Self::RetryableFailure => Self::RetryableFailureAfterRepair,
            Self::RetryableFailureAfterRepair => Self::RetryableFailureAfterRepair,
        }
    }

    fn after_attempted(self) -> bool {
        matches!(self, Self::RetryableFailureAfterRepair)
    }

    fn payload_kind(self) -> ToolDrivenFollowupKind {
        match self {
            Self::RetryableFailure | Self::RetryableFailureAfterRepair => {
                ToolDrivenFollowupKind::ToolFailure
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingToolContinuationDecision {
    Finish {
        continuation_state: Option<ToolDrivenContinuationState>,
    },
    RequestRepair,
    ForceBlockedReply,
}

fn evaluate_missing_tool_continuation(
    expectation: MissingToolContinuationExpectation,
    parsed_reply: &ParsedToolDrivenContinuationReply,
) -> MissingToolContinuationDecision {
    match parsed_reply.state {
        Some(ToolDrivenContinuationState::Continue) => {
            if expectation.after_attempted() {
                MissingToolContinuationDecision::ForceBlockedReply
            } else {
                MissingToolContinuationDecision::RequestRepair
            }
        }
        Some(ToolDrivenContinuationState::Done) => {
            if parsed_reply.reply.is_empty() {
                if expectation.after_attempted() {
                    MissingToolContinuationDecision::ForceBlockedReply
                } else {
                    MissingToolContinuationDecision::RequestRepair
                }
            } else {
                MissingToolContinuationDecision::Finish {
                    continuation_state: Some(ToolDrivenContinuationState::Done),
                }
            }
        }
        Some(ToolDrivenContinuationState::Blocked) => {
            if parsed_reply.reply.is_empty() {
                MissingToolContinuationDecision::ForceBlockedReply
            } else {
                MissingToolContinuationDecision::Finish {
                    continuation_state: Some(ToolDrivenContinuationState::Blocked),
                }
            }
        }
        None => {
            if expectation.after_attempted() {
                MissingToolContinuationDecision::ForceBlockedReply
            } else {
                MissingToolContinuationDecision::RequestRepair
            }
        }
    }
}

fn missing_tool_continuation_blocked_reply(
    expectation: MissingToolContinuationExpectation,
) -> String {
    match expectation.payload_kind() {
        ToolDrivenFollowupKind::ToolFailure => "I couldn't continue because the required retry tool call was never issued. The turn stopped here instead of pretending the retry happened.".to_owned(),
        ToolDrivenFollowupKind::ToolResult => "I couldn't continue because the required follow-up tool call was never issued. The turn stopped here instead of pretending the work completed.".to_owned(),
        ToolDrivenFollowupKind::DiscoveryRecovery => "I couldn't continue because the required follow-up tool call was never issued.".to_owned(),
    }
}

fn build_missing_tool_continuation_repair_messages(
    base_messages: &[Value],
    assistant_reply: &str,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    expectation: MissingToolContinuationExpectation,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    let assistant_reply = assistant_reply.trim();
    if !assistant_reply.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": assistant_reply,
        }));
    }
    let continuation_contract =
        render_tool_followup_continuation_contract(expectation.contract_mode());
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_user_prompt_with_context(
            user_input,
            loop_warning_reason,
            None,
            None,
            Some(continuation_contract.as_str()),
        ),
    }));
    messages
}

#[derive(Debug, Clone)]
struct ProviderReplyLoopState {
    current_preparation: ProviderTurnPreparation,
    current_continue_phase: ProviderTurnContinuePhase,
    remaining_provider_rounds: usize,
    provider_round_index: usize,
    pending_missing_tool_continuation: Option<MissingToolContinuationExpectation>,
}

impl ProviderReplyLoopState {
    fn new(
        preparation: &ProviderTurnPreparation,
        continue_phase: &ProviderTurnContinuePhase,
        remaining_provider_rounds: usize,
    ) -> Self {
        Self {
            current_preparation: preparation.clone(),
            current_continue_phase: continue_phase.clone(),
            remaining_provider_rounds: remaining_provider_rounds.max(1),
            provider_round_index: 0,
            pending_missing_tool_continuation: None,
        }
    }

    fn current_provider_round(&self) -> usize {
        self.provider_round_index.saturating_add(1)
    }
}

#[derive(Debug, Clone)]
enum ReplyLoopDecision {
    FinalizeDirect {
        reply: String,
        latest_tool_payload: Option<ToolDrivenFollowupPayload>,
        continuation_state: Option<ToolDrivenContinuationState>,
    },
    Followup {
        raw_reply: String,
        payload: ToolDrivenFollowupPayload,
        requires_completion_pass: bool,
        loop_warning_reason: Option<String>,
    },
    RepairFollowup {
        raw_reply: String,
        expectation: MissingToolContinuationExpectation,
        loop_warning_reason: Option<String>,
    },
    GuardFollowup {
        raw_reply: String,
        reason: String,
        latest_tool_payload: Option<ToolDrivenFollowupPayload>,
    },
}

fn build_reply_loop_decision(state: &mut ProviderReplyLoopState) -> ReplyLoopDecision {
    match state.current_continue_phase.reply_phase.decision() {
        ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => {
            let latest_tool_payload = tool_driven_followup_payload(
                state.current_continue_phase.lane_execution.had_tool_intents,
                &state.current_continue_phase.lane_execution.turn_result,
            );
            if let Some(reason) = state.current_continue_phase.hard_stop_reason() {
                ReplyLoopDecision::GuardFollowup {
                    raw_reply: reply.clone(),
                    reason: reason.to_owned(),
                    latest_tool_payload,
                }
            } else if let Some(expectation) = state.pending_missing_tool_continuation.take() {
                let parsed_reply = parse_tool_driven_continuation_reply(
                    state
                        .current_continue_phase
                        .lane_execution
                        .assistant_preface
                        .as_str(),
                );
                match evaluate_missing_tool_continuation(expectation, &parsed_reply) {
                    MissingToolContinuationDecision::Finish { continuation_state } => {
                        ReplyLoopDecision::FinalizeDirect {
                            reply: reply.clone(),
                            latest_tool_payload,
                            continuation_state,
                        }
                    }
                    MissingToolContinuationDecision::RequestRepair => {
                        ReplyLoopDecision::RepairFollowup {
                            raw_reply: reply.clone(),
                            expectation,
                            loop_warning_reason: state
                                .current_continue_phase
                                .loop_warning_reason()
                                .map(ToOwned::to_owned),
                        }
                    }
                    MissingToolContinuationDecision::ForceBlockedReply => {
                        ReplyLoopDecision::FinalizeDirect {
                            reply: missing_tool_continuation_blocked_reply(expectation),
                            latest_tool_payload,
                            continuation_state: Some(ToolDrivenContinuationState::Blocked),
                        }
                    }
                }
            } else if let Some(payload) = provider_turn_missing_tool_followup_payload(
                &state.current_continue_phase.lane_execution,
                reply.as_str(),
            ) {
                ReplyLoopDecision::Followup {
                    raw_reply: reply.clone(),
                    payload,
                    requires_completion_pass: true,
                    loop_warning_reason: state
                        .current_continue_phase
                        .loop_warning_reason()
                        .map(ToOwned::to_owned),
                }
            } else if state
                .current_continue_phase
                .lane_execution
                .supports_provider_turn_followup
                && (!state
                    .current_continue_phase
                    .lane_execution
                    .raw_tool_output_requested
                    || state
                        .current_continue_phase
                        .lane_execution
                        .discovery_search_turn
                    || assistant_preface_signals_provider_turn_followup(
                        state
                            .current_continue_phase
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
                    loop_warning_reason: state
                        .current_continue_phase
                        .loop_warning_reason()
                        .map(ToOwned::to_owned),
                }
            } else {
                ReplyLoopDecision::FinalizeDirect {
                    reply: reply.clone(),
                    latest_tool_payload,
                    continuation_state: None,
                }
            }
        }
        ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply,
            payload: followup,
        } => {
            if let Some(reason) = state.current_continue_phase.hard_stop_reason() {
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
                    loop_warning_reason: state
                        .current_continue_phase
                        .loop_warning_reason()
                        .map(ToOwned::to_owned),
                }
            }
        }
    }
}

fn finalize_provider_reply(
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    continue_phase: &ProviderTurnContinuePhase,
    session_id: &str,
    reply: String,
    latest_tool_payload: Option<ToolDrivenFollowupPayload>,
    continuation_state: Option<ToolDrivenContinuationState>,
) -> ResolvedProviderTurn {
    #[cfg(feature = "memory-sqlite")]
    if let Some(latest_tool_payload) = latest_tool_payload.as_ref() {
        persist_active_external_skills_from_followup_payload_if_needed(
            &continue_phase.followup_config,
            session_id,
            latest_tool_payload,
        );
    }

    let checkpoint = continue_phase.checkpoint_with_continuation_state(
        preparation,
        user_input,
        &reply,
        continuation_state,
    );
    ResolvedProviderTurn::persist_reply(
        reply,
        continue_phase.lane_execution.provider_usage.clone(),
        checkpoint,
    )
}

async fn handle_guard_followup_reply<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    binding: ConversationRuntimeBinding<'_>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
    state: &ProviderReplyLoopState,
    raw_reply: String,
    reason: String,
    latest_tool_payload: Option<ToolDrivenFollowupPayload>,
) -> ResolvedProviderTurn {
    #[cfg(feature = "memory-sqlite")]
    if let Some(latest_tool_payload) = latest_tool_payload.as_ref() {
        persist_active_external_skills_from_followup_payload_if_needed(
            &state.current_continue_phase.followup_config,
            session_id,
            latest_tool_payload,
        );
    }

    let guard_messages = build_turn_reply_guard_messages(
        &state.current_preparation.session.messages,
        state
            .current_continue_phase
            .lane_execution
            .assistant_preface
            .as_str(),
        reason.as_str(),
        latest_tool_payload.as_ref(),
        user_input,
    );
    let reply = request_completion_with_raw_fallback(
        runtime,
        &state.current_continue_phase.followup_config,
        &guard_messages,
        binding,
        raw_reply.as_str(),
        retry_progress,
    )
    .await;
    let checkpoint =
        state
            .current_continue_phase
            .checkpoint(preparation, user_input, reply.as_str());
    ResolvedProviderTurn::persist_reply(reply, None, checkpoint)
}

fn provider_turn_missing_tool_followup_payload(
    lane_execution: &ProviderTurnLaneExecution,
    reply_text: &str,
) -> Option<ToolDrivenFollowupPayload> {
    if !lane_execution.supports_provider_turn_followup || lane_execution.had_tool_intents {
        return None;
    }

    missing_tool_call_followup_payload(reply_text).or_else(|| {
        lane_execution
            .malformed_parse_followup_turn
            .then(|| ToolDrivenFollowupPayload::ToolFailure {
                reason: "missing_tool_call_followup: previous provider reply contained malformed tool-call markup instead of a valid tool call. If another tool is required, emit the exact next tool call now instead of malformed tool text.".to_owned(),
                retryable: true,
            })
    })
}

async fn handle_followup_reply_decision<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
    current_provider_round: usize,
    current_preparation: &mut ProviderTurnPreparation,
    current_continue_phase: &mut ProviderTurnContinuePhase,
    remaining_provider_rounds: &mut usize,
    provider_round_index: &mut usize,
    pending_missing_tool_continuation: &mut Option<MissingToolContinuationExpectation>,
    raw_reply: String,
    followup: ToolDrivenFollowupPayload,
    requires_completion_pass: bool,
    loop_warning_reason: Option<String>,
) -> Option<ResolvedProviderTurn> {
    let continuation_expectation = MissingToolContinuationExpectation::from_tool_failure(&followup);
    let provider_continuation_enabled = current_continue_phase
        .lane_execution
        .supports_provider_turn_followup;
    #[cfg(feature = "memory-sqlite")]
    persist_active_external_skills_from_followup_payload_if_needed(
        &current_continue_phase.followup_config,
        session_id,
        &followup,
    );
    let follow_up_messages = build_turn_reply_followup_messages_with_contract(
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
        continuation_expectation.map(MissingToolContinuationExpectation::contract_mode),
    );

    if provider_continuation_enabled && *remaining_provider_rounds > 1 {
        let next_provider_round = current_provider_round.saturating_add(1);
        *remaining_provider_rounds -= 1;
        let initial_estimated_tokens = estimate_tokens_for_messages(
            current_preparation.session.estimated_tokens,
            &current_preparation.session.messages,
        );
        let followup_request_estimated_tokens = estimate_tokens(&follow_up_messages);
        let followup_added_estimated_tokens = initial_estimated_tokens
            .zip(followup_request_estimated_tokens)
            .map(|(initial, followup): (usize, usize)| followup.saturating_sub(initial));
        let followup_preparation = current_preparation.for_followup_messages(follow_up_messages);
        let followup_tool_view =
            match runtime.tool_view(&current_continue_phase.followup_config, session_id, binding) {
                Ok(tool_view) => tool_view,
                Err(_error) => {
                    let checkpoint = current_continue_phase.checkpoint(
                        preparation,
                        user_input,
                        raw_reply.as_str(),
                    );
                    return Some(ResolvedProviderTurn::persist_reply(
                        raw_reply,
                        current_continue_phase.lane_execution.provider_usage.clone(),
                        checkpoint,
                    ));
                }
            };
        let followup_message_count = followup_preparation.session.messages.len();
        let followup_context_estimated_tokens = followup_preparation.session.estimated_tokens;
        let followup_request_event = ConversationTurnPhaseEvent::requesting_followup_provider(
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
                retry_progress.clone(),
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
                let returned_tool_intent_count = turn.tool_intents.len();
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
                            "resolved_to_tool_invoke": followup_result.resolved_to_tool_invoke,
                            "raw_tool_output_requested": current_continue_phase
                                .lane_execution
                                .raw_tool_output_requested,
                        }),
                        binding,
                    )
                    .await;
                }
                if let Some(reply) = turn_loop_state
                    .circuit_breaker_reply(turn_loop_policy, returned_tool_intent_count)
                {
                    return Some(build_turn_loop_circuit_breaker_resolved_turn(
                        preparation,
                        user_input,
                        returned_tool_intent_count,
                        reply,
                    ));
                }
                *current_continue_phase = prepare_provider_turn_continue_phase(
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
                        .supports_provider_turn_followup
                        || provider_continuation_enabled,
                )
                .await;
                *current_preparation = followup_preparation;
                *provider_round_index = provider_round_index.saturating_add(1);
                *pending_missing_tool_continuation = if returned_tool_intent_count == 0 {
                    continuation_expectation
                } else {
                    None
                };
                return None;
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
                let checkpoint =
                    current_continue_phase.checkpoint(preparation, user_input, raw_reply.as_str());
                return Some(ResolvedProviderTurn::persist_reply(
                    raw_reply,
                    current_continue_phase.lane_execution.provider_usage.clone(),
                    checkpoint,
                ));
            }
        }
    }

    if requires_completion_pass {
        let completion_reply = request_completion_with_raw_fallback_detailed(
            runtime,
            &current_continue_phase.followup_config,
            &follow_up_messages,
            binding,
            raw_reply.as_str(),
            retry_progress.clone(),
        )
        .await;
        let (reply, continuation_state) = if let Some(expectation) = continuation_expectation {
            match evaluate_missing_tool_continuation(expectation, &completion_reply) {
                MissingToolContinuationDecision::Finish { continuation_state } => {
                    (completion_reply.reply, continuation_state)
                }
                MissingToolContinuationDecision::RequestRepair => {
                    let repair_expectation = expectation.after_attempt();
                    let repair_messages = build_missing_tool_continuation_repair_messages(
                        &current_preparation.session.messages,
                        completion_reply.reply.as_str(),
                        user_input,
                        loop_warning_reason.as_deref(),
                        repair_expectation,
                    );
                    let repaired_completion_reply = request_completion_with_raw_fallback_detailed(
                        runtime,
                        &current_continue_phase.followup_config,
                        &repair_messages,
                        binding,
                        raw_reply.as_str(),
                        retry_progress.clone(),
                    )
                    .await;
                    match evaluate_missing_tool_continuation(
                        repair_expectation,
                        &repaired_completion_reply,
                    ) {
                        MissingToolContinuationDecision::Finish { continuation_state } => {
                            (repaired_completion_reply.reply, continuation_state)
                        }
                        MissingToolContinuationDecision::RequestRepair
                        | MissingToolContinuationDecision::ForceBlockedReply => (
                            missing_tool_continuation_blocked_reply(repair_expectation),
                            Some(ToolDrivenContinuationState::Blocked),
                        ),
                    }
                }
                MissingToolContinuationDecision::ForceBlockedReply => (
                    missing_tool_continuation_blocked_reply(expectation),
                    Some(ToolDrivenContinuationState::Blocked),
                ),
            }
        } else {
            (completion_reply.reply, completion_reply.state)
        };
        let checkpoint = current_continue_phase.checkpoint_with_continuation_state(
            preparation,
            user_input,
            reply.as_str(),
            continuation_state,
        );
        return Some(ResolvedProviderTurn::persist_reply(reply, None, checkpoint));
    }

    let checkpoint = current_continue_phase.checkpoint(preparation, user_input, raw_reply.as_str());
    Some(ResolvedProviderTurn::persist_reply(
        raw_reply,
        current_continue_phase.lane_execution.provider_usage.clone(),
        checkpoint,
    ))
}

async fn handle_repair_followup_reply<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    turn_loop_policy: &ProviderTurnLoopPolicy,
    turn_loop_state: &mut ProviderTurnLoopState,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    observer: Option<&ConversationTurnObserverHandle>,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
    current_provider_round: usize,
    current_preparation: &mut ProviderTurnPreparation,
    current_continue_phase: &mut ProviderTurnContinuePhase,
    remaining_provider_rounds: &mut usize,
    provider_round_index: &mut usize,
    pending_missing_tool_continuation: &mut Option<MissingToolContinuationExpectation>,
    raw_reply: String,
    expectation: MissingToolContinuationExpectation,
    loop_warning_reason: Option<String>,
) -> Option<ResolvedProviderTurn> {
    if *remaining_provider_rounds <= 1 {
        let reply = missing_tool_continuation_blocked_reply(expectation);
        let checkpoint = current_continue_phase.checkpoint_with_continuation_state(
            preparation,
            user_input,
            reply.as_str(),
            Some(ToolDrivenContinuationState::Blocked),
        );
        return Some(ResolvedProviderTurn::persist_reply(
            reply,
            current_continue_phase.lane_execution.provider_usage.clone(),
            checkpoint,
        ));
    }

    let repair_messages = build_missing_tool_continuation_repair_messages(
        &current_preparation.session.messages,
        raw_reply.as_str(),
        user_input,
        loop_warning_reason.as_deref(),
        expectation.after_attempt(),
    );
    let next_provider_round = current_provider_round.saturating_add(1);
    *remaining_provider_rounds -= 1;
    let repair_preparation = current_preparation.for_followup_messages(repair_messages);
    let repair_tool_view =
        match runtime.tool_view(&current_continue_phase.followup_config, session_id, binding) {
            Ok(tool_view) => tool_view,
            Err(_error) => {
                let reply = missing_tool_continuation_blocked_reply(expectation);
                let checkpoint = current_continue_phase.checkpoint_with_continuation_state(
                    preparation,
                    user_input,
                    reply.as_str(),
                    Some(ToolDrivenContinuationState::Blocked),
                );
                return Some(ResolvedProviderTurn::persist_reply(
                    reply,
                    current_continue_phase.lane_execution.provider_usage.clone(),
                    checkpoint,
                ));
            }
        };
    let repair_request_event = ConversationTurnPhaseEvent::requesting_followup_provider(
        next_provider_round,
        current_continue_phase.lane_execution.lane,
        current_continue_phase.tool_intent_count(),
        repair_preparation.session.messages.len(),
        repair_preparation.session.estimated_tokens,
    );
    observe_turn_phase(observer, repair_request_event);
    emit_prompt_frame_event(
        runtime,
        session_id,
        next_provider_round,
        "followup_repair",
        repair_preparation.session.prompt_frame_summary(),
        binding,
    )
    .await;
    match decide_provider_turn_request_action(
        request_provider_turn_with_observer(
            &current_continue_phase.followup_config,
            runtime,
            session_id,
            repair_preparation.turn_id.as_str(),
            &repair_preparation.session.messages,
            &repair_tool_view,
            binding,
            observer,
            retry_progress,
        )
        .await,
        ProviderErrorMode::Propagate,
    ) {
        ProviderTurnRequestAction::Continue { turn } => {
            let turn = scope_provider_turn_tool_intents(
                turn,
                session_id,
                repair_preparation.turn_id.as_str(),
            );
            let repair_tool_intent_count = turn.tool_intents.len();
            if let Some(reply) =
                turn_loop_state.circuit_breaker_reply(turn_loop_policy, repair_tool_intent_count)
            {
                return Some(build_turn_loop_circuit_breaker_resolved_turn(
                    preparation,
                    user_input,
                    repair_tool_intent_count,
                    reply,
                ));
            }
            *current_continue_phase = prepare_provider_turn_continue_phase(
                &current_continue_phase.followup_config,
                runtime,
                session_id,
                &repair_preparation,
                turn,
                turn_loop_policy,
                turn_loop_state,
                binding,
                ingress,
                observer,
                next_provider_round,
                true,
            )
            .await;
            *current_preparation = repair_preparation;
            *provider_round_index = provider_round_index.saturating_add(1);
            *pending_missing_tool_continuation = if repair_tool_intent_count == 0 {
                Some(expectation.after_attempt())
            } else {
                None
            };
            None
        }
        ProviderTurnRequestAction::FinalizeInlineProviderError { .. }
        | ProviderTurnRequestAction::ReturnError { .. } => {
            let reply = missing_tool_continuation_blocked_reply(expectation);
            let checkpoint = current_continue_phase.checkpoint_with_continuation_state(
                preparation,
                user_input,
                reply.as_str(),
                Some(ToolDrivenContinuationState::Blocked),
            );
            Some(ResolvedProviderTurn::persist_reply(
                reply,
                current_continue_phase.lane_execution.provider_usage.clone(),
                checkpoint,
            ))
        }
    }
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
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> ResolvedProviderTurn {
    let mut state =
        ProviderReplyLoopState::new(preparation, continue_phase, remaining_provider_rounds);

    loop {
        let current_provider_round = state.current_provider_round();
        if state
            .current_continue_phase
            .lane_execution
            .discovery_search_turn
        {
            emit_discovery_first_event(
                runtime,
                session_id,
                "discovery_first_search_round",
                json!({
                    "provider_round": current_provider_round,
                    "search_tool_calls": state.current_continue_phase
                        .lane_execution
                        .search_tool_intents,
                    "raw_tool_output_requested": state.current_continue_phase
                        .lane_execution
                        .raw_tool_output_requested,
                    "initial_estimated_tokens": estimate_tokens_for_messages(
                        state.current_preparation.session.estimated_tokens,
                        &state.current_preparation.session.messages,
                    ),
                }),
                binding,
            )
            .await;
        }

        let reply_decision = build_reply_loop_decision(&mut state);
        match reply_decision {
            ReplyLoopDecision::FinalizeDirect {
                reply,
                latest_tool_payload,
                continuation_state,
            } => {
                return finalize_provider_reply(
                    preparation,
                    user_input,
                    &state.current_continue_phase,
                    session_id,
                    reply,
                    latest_tool_payload,
                    continuation_state,
                );
            }
            ReplyLoopDecision::Followup {
                raw_reply,
                payload: followup,
                requires_completion_pass,
                loop_warning_reason,
            } => {
                if let Some(resolved) = handle_followup_reply_decision(
                    runtime,
                    config,
                    session_id,
                    preparation,
                    user_input,
                    turn_loop_policy,
                    turn_loop_state,
                    binding,
                    ingress,
                    observer,
                    retry_progress.clone(),
                    current_provider_round,
                    &mut state.current_preparation,
                    &mut state.current_continue_phase,
                    &mut state.remaining_provider_rounds,
                    &mut state.provider_round_index,
                    &mut state.pending_missing_tool_continuation,
                    raw_reply,
                    followup,
                    requires_completion_pass,
                    loop_warning_reason,
                )
                .await
                {
                    return resolved;
                }
                continue;
            }
            ReplyLoopDecision::RepairFollowup {
                raw_reply,
                expectation,
                loop_warning_reason,
            } => {
                if let Some(resolved) = handle_repair_followup_reply(
                    runtime,
                    session_id,
                    preparation,
                    user_input,
                    turn_loop_policy,
                    turn_loop_state,
                    binding,
                    ingress,
                    observer,
                    retry_progress.clone(),
                    current_provider_round,
                    &mut state.current_preparation,
                    &mut state.current_continue_phase,
                    &mut state.remaining_provider_rounds,
                    &mut state.provider_round_index,
                    &mut state.pending_missing_tool_continuation,
                    raw_reply,
                    expectation,
                    loop_warning_reason,
                )
                .await
                {
                    return resolved;
                }
                continue;
            }
            ReplyLoopDecision::GuardFollowup {
                raw_reply,
                reason,
                latest_tool_payload,
            } => {
                return handle_guard_followup_reply(
                    runtime,
                    session_id,
                    preparation,
                    user_input,
                    binding,
                    retry_progress.clone(),
                    &state,
                    raw_reply,
                    reason,
                    latest_tool_payload,
                )
                .await;
            }
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn persist_active_external_skills_from_followup_payload_if_needed(
    config: &LoongConfig,
    session_id: &str,
    payload: &ToolDrivenFollowupPayload,
) {
    let ToolDrivenFollowupPayload::ToolResult { text } = payload else {
        return;
    };

    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    let updates =
        active_external_skills::collect_active_external_skills_from_tool_result_text_with_config(
            text,
            &tool_runtime_config,
        );
    if updates.is_empty() {
        return;
    }

    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    let Ok(repo) = SessionRepository::new(&memory_config) else {
        return;
    };
    let Ok(existing_state) =
        active_external_skills::load_persisted_active_external_skills(&repo, session_id)
    else {
        return;
    };
    let Some(merged_state) =
        active_external_skills::merge_active_external_skills(existing_state.clone(), updates)
    else {
        return;
    };
    if existing_state.as_ref() == Some(&merged_state) {
        return;
    }

    let _ = repo.append_event(NewSessionEvent {
        session_id: session_id.to_owned(),
        event_kind: active_external_skills::ACTIVE_EXTERNAL_SKILLS_EVENT_KIND.to_owned(),
        actor_session_id: Some(session_id.to_owned()),
        payload_json: json!({
            "source": "tool_followup",
            "active_external_skills": merged_state,
        }),
    });
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
    build_turn_reply_followup_messages_with_contract(
        base_messages,
        assistant_preface,
        followup,
        tool_request_summary,
        user_input,
        loop_warning_reason,
        None,
    )
}

fn build_turn_reply_followup_messages_with_contract(
    base_messages: &[Value],
    assistant_preface: &str,
    followup: ToolDrivenFollowupPayload,
    tool_request_summary: Option<&str>,
    user_input: &str,
    loop_warning_reason: Option<&str>,
    continuation_contract: Option<ToolDrivenFollowupContractMode>,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
    let continuation_contract =
        continuation_contract.map(render_tool_followup_continuation_contract);
    messages.extend(
        build_tool_driven_followup_tail_with_request_summary_and_contract(
            assistant_preface,
            &followup,
            user_input,
            loop_warning_reason,
            tool_request_summary,
            continuation_contract.as_deref(),
            |label, text| reduce_followup_payload_for_model(label, text).into_owned(),
        ),
    );
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
