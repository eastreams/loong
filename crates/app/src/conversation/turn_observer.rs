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
    pub actual_input_tokens: Option<u32>,
    pub actual_output_tokens: Option<u32>,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
        }
    }

    pub fn completed(
        message_count: usize,
        estimated_tokens: Option<usize>,
        actual_input_tokens: Option<u32>,
        actual_output_tokens: Option<u32>,
    ) -> Self {
        Self {
            phase: ConversationTurnPhase::Completed,
            provider_round: None,
            lane: None,
            tool_call_count: 0,
            message_count: Some(message_count),
            estimated_tokens,
            actual_input_tokens,
            actual_output_tokens,
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
            actual_input_tokens: None,
            actual_output_tokens: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationTurnToolState {
    Running,
    Completed,
    NeedsApproval,
    Denied,
    Failed,
    Interrupted,
}

impl ConversationTurnToolState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::NeedsApproval => "needs_approval",
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnToolEvent {
    pub tool_call_id: String,
    pub tool_name: String,
    pub state: ConversationTurnToolState,
    pub detail: Option<String>,
}

impl ConversationTurnToolEvent {
    pub fn running(tool_call_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Running,
            detail: None,
        }
    }

    pub fn running_with_detail(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Running,
            detail: Some(detail.into()),
        }
    }

    pub fn completed(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Completed,
            detail,
        }
    }

    pub fn needs_approval(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::NeedsApproval,
            detail: Some(detail.into()),
        }
    }

    pub fn denied(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Denied,
            detail: Some(detail.into()),
        }
    }

    pub fn failed(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Failed,
            detail: Some(detail.into()),
        }
    }

    pub fn interrupted(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            state: ConversationTurnToolState::Interrupted,
            detail: Some(detail.into()),
        }
    }
}

pub trait ConversationTurnObserver: Send + Sync {
    fn on_phase(&self, _event: ConversationTurnPhaseEvent) {}

    fn on_tool(&self, _event: ConversationTurnToolEvent) {}

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

    #[test]
    fn map_streaming_callback_data_to_token_event_keeps_tool_call_start_shape() {
        let data = crate::provider::StreamingCallbackData::ToolCallStart {
            index: 1,
            name: "search".to_owned(),
            id: "call_123".to_owned(),
        };
        let event = map_streaming_callback_data_to_token_event(data);
        let tool_call = event
            .delta
            .tool_call
            .expect("tool call delta should be present");

        assert_eq!(event.event_type, "tool_call_start");
        assert_eq!(event.index, Some(1));
        assert_eq!(tool_call.name.as_deref(), Some("search"));
        assert_eq!(tool_call.id.as_deref(), Some("call_123"));
        assert!(tool_call.args.is_none());
    }
}
