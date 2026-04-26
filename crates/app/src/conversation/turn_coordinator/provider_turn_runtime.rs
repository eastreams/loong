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
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> CliResult<ProviderTurn> {
    if let Some(observer) = observer
        && provider_turn_observer_supports_streaming(config, Some(observer))
    {
        let request_started_at = std::time::Instant::now();
        let on_token = build_observer_streaming_token_callback(observer, request_started_at);
        return runtime
            .request_turn_streaming_with_retry_progress(
                config,
                session_id,
                turn_id,
                messages,
                tool_view,
                binding,
                on_token,
                retry_progress,
            )
            .await;
    }

    runtime
        .request_turn_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            binding,
            retry_progress,
        )
        .await
}

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnLanePlan {
    pub(super) decision: LaneDecision,
    pub(super) max_tool_steps: usize,
}

impl ProviderTurnLanePlan {
    pub(super) fn from_user_input(config: &LoongConfig, user_input: &str) -> Self {
        let hybrid_lane_available = config.conversation.hybrid_lane_enabled;
        let safe_lane_plan_available = config.conversation.safe_lane_plan_execution_enabled;
        let use_lane_arbiter = hybrid_lane_available && safe_lane_plan_available;

        let decision = if use_lane_arbiter {
            lane_policy_from_config(config).decide(user_input)
        } else {
            let reason_key = if hybrid_lane_available {
                "safe_lane_plan_disabled"
            } else {
                "hybrid_lane_disabled"
            };
            fast_only_lane_decision(user_input, reason_key)
        };
        let max_tool_steps = match decision.lane {
            ExecutionLane::Fast => config.conversation.fast_lane_max_tool_steps(),
            ExecutionLane::Safe => config.conversation.safe_lane_max_tool_steps(),
        };

        Self {
            decision,
            max_tool_steps,
        }
    }

    pub(super) fn should_use_safe_lane_plan_path(
        &self,
        config: &LoongConfig,
        turn: &ProviderTurn,
    ) -> bool {
        config.conversation.safe_lane_plan_execution_enabled
            && matches!(self.decision.lane, ExecutionLane::Safe)
            && !turn.tool_intents.is_empty()
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnLaneExecution {
    pub(super) lane: ExecutionLane,
    pub(super) assistant_preface: String,
    pub(super) provider_usage: Option<Value>,
    pub(super) had_tool_intents: bool,
    pub(super) tool_request_summary: Option<String>,
    pub(super) discovery_search_turn: bool,
    pub(super) search_tool_intents: usize,
    pub(super) malformed_parse_followup_turn: bool,
    pub(super) supports_provider_turn_followup: bool,
    pub(super) raw_tool_output_requested: bool,
    pub(super) turn_result: TurnResult,
    pub(super) safe_lane_terminal_route: Option<SafeLaneFailureRoute>,
    pub(super) tool_events: Vec<ConversationTurnToolEvent>,
}

impl ProviderTurnLaneExecution {
    pub(super) fn checkpoint(&self) -> TurnLaneExecutionSnapshot {
        TurnLaneExecutionSnapshot {
            lane: self.lane,
            had_tool_intents: self.had_tool_intents,
            tool_request_summary: self.tool_request_summary.clone(),
            raw_tool_output_requested: self.raw_tool_output_requested,
            result_kind: turn_checkpoint_result_kind(&self.turn_result),
            safe_lane_terminal_route: self.safe_lane_terminal_route,
        }
    }

    pub(super) fn reply_phase(&self) -> ToolDrivenReplyPhase {
        ToolDrivenReplyPhase::new(
            self.assistant_preface.as_str(),
            self.had_tool_intents,
            self.raw_tool_output_requested,
            &self.turn_result,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProviderTurnLoopPolicy {
    pub(super) max_total_tool_calls: usize,
    pub(super) max_consecutive_same_tool: usize,
}

impl ProviderTurnLoopPolicy {
    pub(super) fn from_config(config: &LoongConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_total_tool_calls: turn_loop.max_total_tool_calls.max(1),
            max_consecutive_same_tool: turn_loop.max_consecutive_same_tool.max(1),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ProviderTurnLoopState {
    pub(super) total_tool_calls: usize,
    pub(super) consecutive_same_tool: usize,
    pub(super) last_tool_name: Option<String>,
    pub(super) warned_same_tool_key: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) enum ProviderTurnLoopVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

impl ProviderTurnLoopState {
    pub(super) fn circuit_breaker_reply(
        &self,
        policy: &ProviderTurnLoopPolicy,
        next_tool_calls: usize,
    ) -> Option<String> {
        let prospective_total = self.total_tool_calls.saturating_add(next_tool_calls);
        tool_loop_circuit_breaker_reply(prospective_total, policy.max_total_tool_calls)
    }

    pub(super) fn observe_turn(
        &mut self,
        policy: &ProviderTurnLoopPolicy,
        turn: &ProviderTurn,
    ) -> Option<ProviderTurnLoopVerdict> {
        let tool_intent_count = turn.tool_intents.len();
        self.total_tool_calls = self.total_tool_calls.saturating_add(tool_intent_count);
        if tool_intent_count == 0 {
            self.warned_same_tool_key = None;
            return None;
        }

        let tool_name_signature = provider_turn_tool_name_signature(&turn.tool_intents);
        if self.last_tool_name.as_deref() == Some(tool_name_signature.as_str()) {
            self.consecutive_same_tool += 1;
        } else {
            self.last_tool_name = Some(tool_name_signature.clone());
            self.consecutive_same_tool = 1;
            self.warned_same_tool_key = None;
        }

        if self.consecutive_same_tool < policy.max_consecutive_same_tool {
            self.warned_same_tool_key = None;
            return Some(ProviderTurnLoopVerdict::Continue);
        }

        let reason_key = format!("consecutive_same_tool:{tool_name_signature}");
        let reason = format!(
            "consecutive_same_tool: {tool_name_signature} called {} times in a row (limit={})",
            self.consecutive_same_tool, policy.max_consecutive_same_tool
        );

        if self.warned_same_tool_key.as_deref() == Some(reason_key.as_str()) {
            Some(ProviderTurnLoopVerdict::HardStop { reason })
        } else {
            self.warned_same_tool_key = Some(reason_key);
            Some(ProviderTurnLoopVerdict::InjectWarning { reason })
        }
    }
}

pub(super) fn provider_turn_tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnContinuePhase {
    request: TurnCheckpointRequest,
    pub(super) lane_execution: ProviderTurnLaneExecution,
    pub(super) reply_phase: ToolDrivenReplyPhase,
    pub(super) loop_verdict: Option<ProviderTurnLoopVerdict>,
    pub(super) followup_config: LoongConfig,
    pub(super) ingress: Option<ConversationIngressContext>,
}

impl ProviderTurnContinuePhase {
    pub(super) fn new(
        tool_intents: usize,
        lane_execution: ProviderTurnLaneExecution,
        loop_verdict: Option<ProviderTurnLoopVerdict>,
        followup_config: LoongConfig,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let reply_phase = lane_execution.reply_phase();
        Self {
            request: TurnCheckpointRequest::Continue { tool_intents },
            lane_execution,
            reply_phase,
            loop_verdict,
            followup_config,
            ingress: ingress.cloned(),
        }
    }

    pub(super) fn checkpoint(
        &self,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        reply: &str,
    ) -> TurnCheckpointSnapshot {
        self.checkpoint_with_continuation_state(preparation, user_input, reply, None)
    }

    pub(super) fn checkpoint_with_continuation_state(
        &self,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        reply: &str,
        continuation_state: Option<ToolDrivenContinuationState>,
    ) -> TurnCheckpointSnapshot {
        let reply_checkpoint = if continuation_state.is_some() {
            TurnReplyCheckpoint::from_phase_with_continuation_state(
                &self.reply_phase,
                continuation_state,
            )
        } else {
            TurnReplyCheckpoint::from_phase(&self.reply_phase)
        };
        build_resolved_provider_checkpoint(
            preparation,
            user_input,
            Some(reply),
            self.request.clone(),
            Some(self.lane_execution.checkpoint()),
            Some(reply_checkpoint),
            TurnFinalizationCheckpoint::persist_reply(ReplyPersistenceMode::Success),
        )
    }

    pub(super) fn tool_intent_count(&self) -> usize {
        match self.request {
            TurnCheckpointRequest::Continue { tool_intents } => tool_intents,
            TurnCheckpointRequest::FinalizeInlineProviderError
            | TurnCheckpointRequest::ReturnError => 0,
        }
    }

    pub(super) fn loop_warning_reason(&self) -> Option<&str> {
        match self.loop_verdict.as_ref() {
            Some(ProviderTurnLoopVerdict::InjectWarning { reason }) => Some(reason.as_str()),
            _ => None,
        }
    }

    pub(super) fn hard_stop_reason(&self) -> Option<&str> {
        match self.loop_verdict.as_ref() {
            Some(ProviderTurnLoopVerdict::HardStop { reason }) => Some(reason.as_str()),
            _ => None,
        }
    }

    pub(super) async fn resolve<R: ConversationRuntime + ?Sized>(
        &self,
        runtime: &R,
        session_id: &str,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
        turn_loop_policy: &ProviderTurnLoopPolicy,
        turn_loop_state: &mut ProviderTurnLoopState,
        remaining_provider_rounds: usize,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> ResolvedProviderTurn {
        resolve_provider_turn_reply(
            runtime,
            &self.followup_config,
            session_id,
            preparation,
            self,
            user_input,
            turn_loop_policy,
            turn_loop_state,
            remaining_provider_rounds,
            binding,
            self.ingress.as_ref(),
            observer,
            retry_progress,
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedProviderTurn {
    PersistReply(ResolvedProviderReply),
    ReturnError(ResolvedProviderError),
}

impl ResolvedProviderTurn {
    pub(super) fn persist_reply(
        reply: String,
        usage: Option<Value>,
        checkpoint: TurnCheckpointSnapshot,
    ) -> Self {
        Self::PersistReply(ResolvedProviderReply {
            reply,
            usage,
            checkpoint,
        })
    }

    pub(super) fn return_error(error: String, checkpoint: TurnCheckpointSnapshot) -> Self {
        Self::ReturnError(ResolvedProviderError { error, checkpoint })
    }

    #[cfg(test)]
    pub(super) fn checkpoint(&self) -> &TurnCheckpointSnapshot {
        match self {
            Self::PersistReply(reply) => &reply.checkpoint,
            Self::ReturnError(error) => &error.checkpoint,
        }
    }

    pub(super) fn terminal_phase<'a>(
        &'a self,
        session: &ProviderTurnSessionState,
    ) -> ProviderTurnTerminalPhase<'a> {
        match self {
            Self::PersistReply(reply) => {
                ProviderTurnTerminalPhase::PersistReply(ProviderTurnPersistReplyPhase {
                    checkpoint: &reply.checkpoint,
                    tail_phase: ProviderTurnReplyTailPhase::from_session(
                        session,
                        reply.reply.as_str(),
                    ),
                    usage: reply.usage.clone(),
                })
            }
            Self::ReturnError(error) => {
                ProviderTurnTerminalPhase::ReturnError(ProviderTurnReturnErrorPhase {
                    checkpoint: &error.checkpoint,
                    error: error.error.as_str(),
                })
            }
        }
    }

    #[cfg(test)]
    pub(super) fn reply_text(&self) -> Option<&str> {
        match self {
            Self::PersistReply(reply) => Some(reply.reply.as_str()),
            Self::ReturnError(_) => None,
        }
    }

    pub(super) fn provider_error_text(&self) -> Option<&str> {
        match self {
            Self::PersistReply(reply) => provider_error_reply_body(reply.reply.as_str()),
            Self::ReturnError(error) => Some(error.error.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedProviderReply {
    pub(super) reply: String,
    pub(super) usage: Option<Value>,
    pub(super) checkpoint: TurnCheckpointSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedProviderError {
    pub(super) error: String,
    pub(super) checkpoint: TurnCheckpointSnapshot,
}

#[derive(Debug)]
pub(super) enum ProviderTurnTerminalPhase<'a> {
    PersistReply(ProviderTurnPersistReplyPhase<'a>),
    ReturnError(ProviderTurnReturnErrorPhase<'a>),
}

impl<'a> ProviderTurnTerminalPhase<'a> {
    pub(super) async fn apply<R: ConversationRuntime + ?Sized>(
        self,
        config: &LoongConfig,
        runtime: &R,
        session_id: &str,
        user_input: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ConversationTurnOutcome> {
        match self {
            Self::PersistReply(phase) => {
                finalize_provider_turn_reply(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    &phase.tail_phase,
                    phase.usage,
                    phase.checkpoint,
                    binding,
                )
                .await
            }
            Self::ReturnError(phase) => {
                persist_resolved_provider_error_checkpoint(
                    runtime,
                    session_id,
                    phase.checkpoint,
                    binding,
                )
                .await?;
                Err(phase.error.to_owned())
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct ProviderTurnPersistReplyPhase<'a> {
    pub(super) checkpoint: &'a TurnCheckpointSnapshot,
    pub(super) tail_phase: ProviderTurnReplyTailPhase,
    pub(super) usage: Option<Value>,
}

#[derive(Debug)]
pub(super) struct ProviderTurnReturnErrorPhase<'a> {
    pub(super) checkpoint: &'a TurnCheckpointSnapshot,
    pub(super) error: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProviderTurnRequestTerminalPhase {
    PersistInlineProviderError { reply: String },
    ReturnError { error: String },
}

impl ProviderTurnRequestTerminalPhase {
    pub(super) fn persist_inline_provider_error(reply: String) -> Self {
        Self::PersistInlineProviderError { reply }
    }

    pub(super) fn return_error(error: String) -> Self {
        Self::ReturnError { error }
    }

    pub(super) fn resolve(
        self,
        preparation: &ProviderTurnPreparation,
        user_input: &str,
    ) -> ResolvedProviderTurn {
        match self {
            Self::PersistInlineProviderError { reply } => {
                let checkpoint = build_resolved_provider_checkpoint(
                    preparation,
                    user_input,
                    Some(reply.as_str()),
                    TurnCheckpointRequest::FinalizeInlineProviderError,
                    None,
                    None,
                    TurnFinalizationCheckpoint::persist_reply(
                        ReplyPersistenceMode::InlineProviderError,
                    ),
                );
                ResolvedProviderTurn::persist_reply(reply, None, checkpoint)
            }
            Self::ReturnError { error } => {
                let checkpoint = build_resolved_provider_checkpoint(
                    preparation,
                    user_input,
                    None,
                    TurnCheckpointRequest::ReturnError,
                    None,
                    None,
                    TurnFinalizationCheckpoint::ReturnError,
                );
                ResolvedProviderTurn::return_error(error, checkpoint)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct SafeLaneTurnOutcome {
    pub(super) result: TurnResult,
    pub(super) terminal_route: Option<SafeLaneFailureRoute>,
}

impl SafeLaneTurnOutcome {
    pub(super) fn without_terminal_route(result: TurnResult) -> Self {
        Self {
            result,
            terminal_route: None,
        }
    }

    pub(super) fn with_terminal_route(
        result: TurnResult,
        terminal_route: SafeLaneFailureRoute,
    ) -> Self {
        Self {
            result,
            terminal_route: Some(terminal_route),
        }
    }
}

pub(super) fn build_resolved_provider_checkpoint(
    preparation: &ProviderTurnPreparation,
    user_input: &str,
    reply_text: Option<&str>,
    request: TurnCheckpointRequest,
    lane: Option<TurnLaneExecutionSnapshot>,
    reply: Option<TurnReplyCheckpoint>,
    finalization: TurnFinalizationCheckpoint,
) -> TurnCheckpointSnapshot {
    TurnCheckpointSnapshot {
        identity: reply_text
            .map(|assistant_reply| TurnCheckpointIdentity::from_turn(user_input, assistant_reply)),
        preparation: preparation.checkpoint(),
        request,
        lane,
        reply,
        finalization,
    }
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
                    config
                        .conversation
                        .turn_loop
                        .max_discovery_followup_rounds
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
