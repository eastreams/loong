use std::sync::Arc;

use crate::acp::{StreamingTokenEvent, TokenDelta, ToolCallDelta};

use super::lane_arbiter::ExecutionLane;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTurnPhase {
    Preparing,
    ContextReady,
    RequestingProvider,
    RunningTools,
    RequestingFollowupProvider,
    FinalizingReply,
    Completed,
    Failed,
}

impl ConversationTurnPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::ContextReady => "context_ready",
            Self::RequestingProvider => "requesting_provider",
            Self::RunningTools => "running_tools",
            Self::RequestingFollowupProvider => "requesting_followup_provider",
            Self::FinalizingReply => "finalizing_reply",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnPhaseEvent {
    pub phase: ConversationTurnPhase,
    pub provider_round: Option<usize>,
    pub lane: Option<ExecutionLane>,
    pub tool_call_count: usize,
    pub message_count: Option<usize>,
    pub estimated_tokens: Option<usize>,
}

impl ConversationTurnPhaseEvent {
    pub fn preparing() -> Self {
        Self {
            phase: ConversationTurnPhase::Preparing,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: None,
            estimated_tokens: None,
        }
    }

    pub fn context_ready(message_count: usize, estimated_tokens: Option<usize>) -> Self {
        Self {
            phase: ConversationTurnPhase::ContextReady,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: Some(message_count),
            estimated_tokens,
        }
    }

    pub fn requesting_provider(
        provider_round: usize,
        message_count: usize,
        estimated_tokens: Option<usize>,
    ) -> Self {
        Self {
            phase: ConversationTurnPhase::RequestingProvider,
            provider_round: Some(provider_round),
            lane: None,
            tool_call_count: 0,
            message_count: Some(message_count),
            estimated_tokens,
        }
    }

    pub fn running_tools(
        provider_round: usize,
        lane: ExecutionLane,
        tool_call_count: usize,
    ) -> Self {
        Self {
            phase: ConversationTurnPhase::RunningTools,
            provider_round: Some(provider_round),
            lane: Some(lane),
            tool_call_count,
            message_count: None,
            estimated_tokens: None,
        }
    }

    pub fn requesting_followup_provider(
        provider_round: usize,
        lane: ExecutionLane,
        tool_call_count: usize,
        message_count: usize,
        estimated_tokens: Option<usize>,
    ) -> Self {
        Self {
            phase: ConversationTurnPhase::RequestingFollowupProvider,
            provider_round: Some(provider_round),
            lane: Some(lane),
            tool_call_count,
            message_count: Some(message_count),
            estimated_tokens,
        }
    }

    pub fn finalizing_reply(message_count: usize, estimated_tokens: Option<usize>) -> Self {
        Self {
            phase: ConversationTurnPhase::FinalizingReply,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: Some(message_count),
            estimated_tokens,
        }
    }

    pub fn completed(message_count: usize, estimated_tokens: Option<usize>) -> Self {
        Self {
            phase: ConversationTurnPhase::Completed,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: Some(message_count),
            estimated_tokens,
        }
    }

    pub fn failed() -> Self {
        Self {
            phase: ConversationTurnPhase::Failed,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: None,
            estimated_tokens: None,
        }
    }
}

pub trait ConversationTurnObserver: Send + Sync {
    fn on_phase(&self, _event: ConversationTurnPhaseEvent) {}

    fn on_streaming_token(&self, _event: StreamingTokenEvent) {}
}

pub type ConversationTurnObserverHandle = Arc<dyn ConversationTurnObserver>;

pub(crate) fn build_observer_streaming_token_callback(
    observer: &ConversationTurnObserverHandle,
) -> crate::provider::StreamingTokenCallback {
    let observer = Arc::clone(observer);
    let callback = move |data: crate::provider::StreamingCallbackData| {
        let event = map_streaming_callback_data_to_token_event(data);
        observer.on_streaming_token(event);
    };
    Some(Arc::new(callback))
}

pub(crate) fn map_streaming_callback_data_to_token_event(
    data: crate::provider::StreamingCallbackData,
) -> StreamingTokenEvent {
    match data {
        crate::provider::StreamingCallbackData::Text { text } => StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: TokenDelta {
                text: Some(text),
                tool_call: None,
            },
            index: None,
        },
        crate::provider::StreamingCallbackData::ToolCallStart { index, name, id } => {
            let tool_call = ToolCallDelta {
                name: Some(name),
                args: None,
                id: Some(id),
            };
            let delta = TokenDelta {
                text: None,
                tool_call: Some(tool_call),
            };
            StreamingTokenEvent {
                event_type: "tool_call_start".to_owned(),
                delta,
                index: Some(index),
            }
        }
        crate::provider::StreamingCallbackData::ToolCallInput {
            index,
            partial_json,
        } => {
            let tool_call = ToolCallDelta {
                name: None,
                args: Some(partial_json),
                id: None,
            };
            let delta = TokenDelta {
                text: None,
                tool_call: Some(tool_call),
            };
            StreamingTokenEvent {
                event_type: "tool_call_input_delta".to_owned(),
                delta,
                index: Some(index),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_streaming_callback_data_to_token_event_keeps_text_delta_shape() {
        let data = crate::provider::StreamingCallbackData::Text {
            text: "hello".to_owned(),
        };
        let event = map_streaming_callback_data_to_token_event(data);

        assert_eq!(event.event_type, "text_delta");
        assert_eq!(event.delta.text.as_deref(), Some("hello"));
        assert!(event.delta.tool_call.is_none());
        assert!(event.index.is_none());
    }

    #[test]
    fn map_streaming_callback_data_to_token_event_keeps_tool_call_delta_shape() {
        let data = crate::provider::StreamingCallbackData::ToolCallInput {
            index: 2,
            partial_json: "{\"query\":\"rust\"}".to_owned(),
        };
        let event = map_streaming_callback_data_to_token_event(data);
        let tool_call = event
            .delta
            .tool_call
            .expect("tool call delta should be present");

        assert_eq!(event.event_type, "tool_call_input_delta");
        assert_eq!(event.index, Some(2));
        assert_eq!(tool_call.args.as_deref(), Some("{\"query\":\"rust\"}"));
        assert!(tool_call.name.is_none());
        assert!(tool_call.id.is_none());
    }
}
