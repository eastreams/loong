use serde_json::{Value, json};

use crate::CliResult;

use super::super::config::LoongConfig;
use super::persistence::persist_reply_turns_with_mode;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_engine::{
    DefaultAppToolDispatcher, ProviderTurn, TurnEngine, TurnResult, TurnValidation,
};
use super::turn_loop_followup::{
    FollowupPayloadBudget, append_repeated_tool_guard_followup_messages,
    append_tool_driven_followup_messages, round_tool_payload_context,
};
use super::turn_loop_state::{
    RoundFollowup, RoundKernelDecision, RoundKernelEvaluation, ToolLoopSupervisor, TurnLoopPolicy,
    TurnLoopSessionState, TurnLoopTerminalAction, tool_name_signature,
    tool_round_has_observable_outcome,
};
use super::turn_shared::{
    ReplyPersistenceMode, request_completion_with_raw_fallback, user_requested_raw_tool_output,
};
use super::{
    FAST_LANE_PARALLEL_TOOL_EXECUTION_ENABLED, FAST_LANE_PARALLEL_TOOL_EXECUTION_MAX_IN_FLIGHT,
    TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
};

pub(super) async fn resolve_round_kernel_terminal_action<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    session: &mut TurnLoopSessionState,
    user_input: &str,
    decision: RoundKernelDecision,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<Option<TurnLoopTerminalAction>> {
    match decision {
        RoundKernelDecision::ContinueWithFollowup(followup) => {
            append_round_followup_messages(session, user_input, followup);
            Ok(None)
        }
        RoundKernelDecision::FinalizeDirect { reply } => {
            Ok(Some(TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode: ReplyPersistenceMode::Success,
            }))
        }
        RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup,
        } => {
            append_round_followup_messages(session, user_input, followup);
            let reply = request_completion_with_raw_fallback(
                runtime,
                config,
                &session.messages,
                binding,
                raw_reply.as_str(),
                None,
            )
            .await;
            Ok(Some(TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode: ReplyPersistenceMode::Success,
            }))
        }
    }
}

pub(super) async fn apply_turn_loop_terminal_action<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    action: TurnLoopTerminalAction,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    match action {
        TurnLoopTerminalAction::PersistReply {
            reply,
            persistence_mode,
        } => {
            persist_reply_turns_with_mode(
                runtime,
                session_id,
                user_input,
                &reply,
                persistence_mode,
                binding,
            )
            .await?;
            Ok(reply)
        }
        TurnLoopTerminalAction::ReturnError { error } => Err(error),
    }
}

pub(super) fn initialize_turn_loop_session(
    mut messages: Vec<Value>,
    user_input: &str,
    policy: &TurnLoopPolicy,
) -> TurnLoopSessionState {
    messages.push(json!({
        "role": "user",
        "content": user_input,
    }));
    TurnLoopSessionState {
        messages,
        raw_tool_output_requested: user_requested_raw_tool_output(user_input),
        last_raw_reply: String::new(),
        loop_supervisor: ToolLoopSupervisor::default(),
        followup_payload_budget: FollowupPayloadBudget::new(
            policy.max_followup_tool_payload_chars,
            policy.max_followup_tool_payload_chars_total,
        ),
        total_tool_calls: 0,
    }
}

pub(super) async fn evaluate_round_kernel(
    _config: &LoongConfig,
    policy: &TurnLoopPolicy,
    turn: &ProviderTurn,
    session_context: &super::runtime::SessionContext,
    app_dispatcher: &DefaultAppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    loop_supervisor: &mut ToolLoopSupervisor,
) -> RoundKernelEvaluation {
    let had_tool_intents = !turn.tool_intents.is_empty();
    let current_tool_name_signature =
        had_tool_intents.then(|| tool_name_signature(&turn.tool_intents));

    let engine = TurnEngine::with_parallel_tool_execution(
        policy.max_tool_steps_per_round,
        TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
        FAST_LANE_PARALLEL_TOOL_EXECUTION_ENABLED,
        FAST_LANE_PARALLEL_TOOL_EXECUTION_MAX_IN_FLIGHT,
    );
    let (turn_result, _turn_trace) = match engine.validate_turn_in_context(turn, session_context) {
        Ok(TurnValidation::FinalText(text)) => (TurnResult::FinalText(text), None),
        Err(failure) => (TurnResult::ToolDenied(failure), None),
        Ok(TurnValidation::ToolExecutionRequired) => {
            engine
                .execute_turn_in_context_with_trace(
                    turn,
                    session_context,
                    app_dispatcher,
                    binding,
                    None,
                    None,
                )
                .await
        }
    };
    let loop_verdict = if let Some(name_signature) = current_tool_name_signature.as_deref() {
        tool_round_has_observable_outcome(&turn_result)
            .then(|| loop_supervisor.observe_round(policy, name_signature))
    } else {
        None
    };

    RoundKernelEvaluation {
        assistant_preface: turn.assistant_text.clone(),
        had_tool_intents,
        tool_request_summary: None,
        turn_result,
        loop_verdict,
    }
}

fn append_round_followup_messages(
    session: &mut TurnLoopSessionState,
    user_input: &str,
    followup: RoundFollowup,
) {
    match followup {
        RoundFollowup::Tool {
            assistant_preface,
            payload,
            tool_request_summary,
            loop_warning_reason,
        } => append_tool_driven_followup_messages(
            &mut session.messages,
            assistant_preface.as_str(),
            &payload,
            user_input,
            &mut session.followup_payload_budget,
            loop_warning_reason.as_deref(),
            tool_request_summary.as_deref(),
        ),
        RoundFollowup::Guard {
            assistant_preface,
            reason,
            latest_tool_payload,
        } => append_repeated_tool_guard_followup_messages(
            &mut session.messages,
            assistant_preface.as_str(),
            reason.as_str(),
            user_input,
            latest_tool_payload.as_ref().map(round_tool_payload_context),
            &mut session.followup_payload_budget,
        ),
    }
}
