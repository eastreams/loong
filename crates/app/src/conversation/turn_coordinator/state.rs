use super::*;
use crate::conversation::{
    FAST_LANE_MAX_TOOL_STEPS_PER_TURN, SAFE_LANE_MAX_TOOL_STEPS_PER_TURN,
    TURN_LOOP_MAX_CONSECUTIVE_SAME_TOOL, TURN_LOOP_MAX_TOTAL_TOOL_CALLS,
};

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnLanePlan {
    pub(super) decision: LaneDecision,
    pub(super) max_tool_steps: usize,
}

impl ProviderTurnLanePlan {
    pub(super) fn from_user_input(config: &LoongConfig, user_input: &str) -> Self {
        let decision = lane_policy_from_config(config).decide(user_input);
        let max_tool_steps = match decision.lane {
            ExecutionLane::Fast => FAST_LANE_MAX_TOOL_STEPS_PER_TURN,
            ExecutionLane::Safe => SAFE_LANE_MAX_TOOL_STEPS_PER_TURN,
        };

        Self {
            decision,
            max_tool_steps,
        }
    }

    pub(super) fn should_use_safe_lane_plan_path(
        &self,
        _config: &LoongConfig,
        turn: &ProviderTurn,
    ) -> bool {
        matches!(self.decision.lane, ExecutionLane::Safe) && !turn.tool_intents.is_empty()
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
    pub(super) fn from_config(_config: &LoongConfig) -> Self {
        Self {
            max_total_tool_calls: TURN_LOOP_MAX_TOTAL_TOOL_CALLS,
            max_consecutive_same_tool: TURN_LOOP_MAX_CONSECUTIVE_SAME_TOOL,
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
    pub(super) request: TurnCheckpointRequest,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationTurnOutcome {
    pub reply: String,
    pub usage: Option<Value>,
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
