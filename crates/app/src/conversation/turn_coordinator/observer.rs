use super::*;
use crate::conversation::turn_observer::build_observer_streaming_token_callback;

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
