use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc::UnboundedSender;

use crate::acp::StreamingTokenEvent;
use crate::conversation::{
    ConversationTurnObserver, ConversationTurnObserverHandle, ConversationTurnPhase,
    ConversationTurnPhaseEvent, ConversationTurnToolEvent, ConversationTurnToolState,
};

use super::events::UiEvent;

struct ObserverState {
    tool_start_times: HashMap<String, Instant>,
    latest_phase: String,
}

impl ObserverState {
    fn new() -> Self {
        Self {
            tool_start_times: HashMap::new(),
            latest_phase: String::new(),
        }
    }
}

pub(super) struct TuiObserver {
    tx: UnboundedSender<UiEvent>,
    state: Mutex<ObserverState>,
}

impl TuiObserver {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, ObserverState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ConversationTurnObserver for TuiObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        let phase_str = format!("{:?}", event.phase);
        let iteration = event.provider_round.unwrap_or(0) as u32;

        {
            let mut state = self.lock_state();
            state.latest_phase = phase_str.clone();
        }

        let _ = self.tx.send(UiEvent::PhaseChange {
            phase: phase_str,
            iteration,
            action: String::new(),
        });

        if event.phase == ConversationTurnPhase::Completed {
            let _ = self.tx.send(UiEvent::ResponseDone {
                input_tokens: event.estimated_tokens.unwrap_or(0) as u32,
                output_tokens: 0,
            });
        }
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        match event.state {
            ConversationTurnToolState::Running => {
                {
                    let mut state = self.lock_state();
                    state
                        .tool_start_times
                        .insert(event.tool_call_id.clone(), Instant::now());
                }

                let _ = self.tx.send(UiEvent::ToolStart {
                    tool_id: event.tool_call_id,
                    tool_name: event.tool_name,
                    args_preview: event.detail.unwrap_or_default(),
                });
            }

            ConversationTurnToolState::Completed
            | ConversationTurnToolState::Failed
            | ConversationTurnToolState::Interrupted => {
                let duration_ms = {
                    let mut state = self.lock_state();
                    state
                        .tool_start_times
                        .remove(&event.tool_call_id)
                        .map(|start| start.elapsed().as_millis().min(u32::MAX as u128) as u32)
                        .unwrap_or(0)
                };

                let success = event.state == ConversationTurnToolState::Completed;

                let _ = self.tx.send(UiEvent::ToolDone {
                    tool_id: event.tool_call_id,
                    success,
                    output: event.detail.unwrap_or_default(),
                    duration_ms,
                });
            }

            ConversationTurnToolState::NeedsApproval => {
                let question = format!(
                    "Tool `{}` requires approval: {}",
                    event.tool_name,
                    event.detail.as_deref().unwrap_or("(no details)")
                );

                let _ = self.tx.send(UiEvent::ClarifyRequest {
                    question,
                    choices: vec!["approve".to_owned(), "deny".to_owned()],
                });
            }

            ConversationTurnToolState::Denied => {
                let duration_ms = {
                    let mut state = self.lock_state();
                    state
                        .tool_start_times
                        .remove(&event.tool_call_id)
                        .map(|start| start.elapsed().as_millis().min(u32::MAX as u128) as u32)
                        .unwrap_or(0)
                };

                let _ = self.tx.send(UiEvent::ToolDone {
                    tool_id: event.tool_call_id,
                    success: false,
                    output: event.detail.unwrap_or_else(|| "denied".to_owned()),
                    duration_ms,
                });
            }
        }
    }

    fn on_streaming_token(&self, event: StreamingTokenEvent) {
        match event.event_type.as_str() {
            "text_delta" => {
                if let Some(content) = event.delta.text {
                    let _ = self.tx.send(UiEvent::Token {
                        content,
                        is_thinking: false,
                    });
                }
            }
            "thinking_delta" => {
                if let Some(content) = event.delta.text {
                    let _ = self.tx.send(UiEvent::Token {
                        content,
                        is_thinking: true,
                    });
                }
            }
            _ => {}
        }
    }
}

pub(super) fn build_tui_observer(tx: UnboundedSender<UiEvent>) -> ConversationTurnObserverHandle {
    Arc::new(TuiObserver {
        tx,
        state: Mutex::new(ObserverState::new()),
    })
}

#[cfg(test)]
#[allow(clippy::wildcard_enum_match_arm)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn setup() -> (
        mpsc::UnboundedReceiver<UiEvent>,
        ConversationTurnObserverHandle,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let observer = build_tui_observer(tx);
        (rx, observer)
    }

    #[test]
    fn phase_event_sends_phase_change() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
            2,
            10,
            Some(500),
        ));

        let event = rx.try_recv().expect("should receive PhaseChange");
        match event {
            UiEvent::PhaseChange {
                phase,
                iteration,
                action,
            } => {
                assert_eq!(phase, "RequestingProvider");
                assert_eq!(iteration, 2);
                assert!(action.is_empty());
            }
            other => panic!("expected PhaseChange, got {:?}", other),
        }

        assert!(rx.try_recv().is_err(), "no extra events expected");
    }

    #[test]
    fn completed_phase_sends_response_done() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::completed(12, Some(1500)));

        let phase_event = rx.try_recv().expect("should receive PhaseChange");
        match phase_event {
            UiEvent::PhaseChange { phase, .. } => assert_eq!(phase, "Completed"),
            other => panic!("expected PhaseChange, got {:?}", other),
        }

        let done_event = rx.try_recv().expect("should receive ResponseDone");
        match done_event {
            UiEvent::ResponseDone {
                input_tokens,
                output_tokens,
            } => {
                assert_eq!(input_tokens, 1500);
                assert_eq!(output_tokens, 0);
            }
            other => panic!("expected ResponseDone, got {:?}", other),
        }
    }

    #[test]
    fn tool_lifecycle_start_then_complete_with_duration() {
        let (mut rx, observer) = setup();

        observer.on_tool(ConversationTurnToolEvent::running("call_1", "search"));

        let start_event = rx.try_recv().expect("should receive ToolStart");
        match start_event {
            UiEvent::ToolStart {
                tool_id,
                tool_name,
                args_preview,
            } => {
                assert_eq!(tool_id, "call_1");
                assert_eq!(tool_name, "search");
                assert!(args_preview.is_empty());
            }
            other => panic!("expected ToolStart, got {:?}", other),
        }

        // Simulate a short delay so duration is >= 0
        observer.on_tool(ConversationTurnToolEvent::completed(
            "call_1",
            "search",
            Some("found 3 results".to_owned()),
        ));

        let done_event = rx.try_recv().expect("should receive ToolDone");
        match done_event {
            UiEvent::ToolDone {
                tool_id,
                success,
                output,
                duration_ms,
            } => {
                assert_eq!(tool_id, "call_1");
                assert!(success);
                assert_eq!(output, "found 3 results");
                // Duration should be non-negative (we just check it doesn't panic)
                let _ = duration_ms;
            }
            other => panic!("expected ToolDone, got {:?}", other),
        }
    }

    #[test]
    fn tool_failed_reports_failure() {
        let (mut rx, observer) = setup();

        observer.on_tool(ConversationTurnToolEvent::running("call_2", "write_file"));
        let _ = rx.try_recv(); // consume ToolStart

        observer.on_tool(ConversationTurnToolEvent::failed(
            "call_2",
            "write_file",
            "permission denied",
        ));

        let done_event = rx.try_recv().expect("should receive ToolDone");
        match done_event {
            UiEvent::ToolDone {
                success, output, ..
            } => {
                assert!(!success);
                assert_eq!(output, "permission denied");
            }
            other => panic!("expected ToolDone, got {:?}", other),
        }
    }

    #[test]
    fn tool_interrupted_reports_failure() {
        let (mut rx, observer) = setup();

        observer.on_tool(ConversationTurnToolEvent::running("call_3", "shell"));
        let _ = rx.try_recv(); // consume ToolStart

        observer.on_tool(ConversationTurnToolEvent::interrupted(
            "call_3",
            "shell",
            "cancelled by user",
        ));

        let done_event = rx.try_recv().expect("should receive ToolDone");
        match done_event {
            UiEvent::ToolDone {
                success, output, ..
            } => {
                assert!(!success);
                assert_eq!(output, "cancelled by user");
            }
            other => panic!("expected ToolDone, got {:?}", other),
        }
    }

    #[test]
    fn needs_approval_sends_clarify_request() {
        let (mut rx, observer) = setup();

        observer.on_tool(ConversationTurnToolEvent::needs_approval(
            "call_4",
            "write_file",
            "writing to /etc/hosts",
        ));

        let event = rx.try_recv().expect("should receive ClarifyRequest");
        match event {
            UiEvent::ClarifyRequest { question, choices } => {
                assert!(question.contains("write_file"));
                assert!(question.contains("writing to /etc/hosts"));
                assert_eq!(choices, vec!["approve", "deny"]);
            }
            other => panic!("expected ClarifyRequest, got {:?}", other),
        }
    }

    #[test]
    fn streaming_text_delta_sends_token() {
        let (mut rx, observer) = setup();

        let event = StreamingTokenEvent {
            event_type: "text_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("hello world".to_owned()),
                tool_call: None,
            },
            index: None,
        };

        observer.on_streaming_token(event);

        let ui_event = rx.try_recv().expect("should receive Token");
        match ui_event {
            UiEvent::Token {
                content,
                is_thinking,
            } => {
                assert_eq!(content, "hello world");
                assert!(!is_thinking);
            }
            other => panic!("expected Token, got {:?}", other),
        }
    }

    #[test]
    fn streaming_thinking_delta_sends_thinking_token() {
        let (mut rx, observer) = setup();

        let event = StreamingTokenEvent {
            event_type: "thinking_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: Some("let me consider".to_owned()),
                tool_call: None,
            },
            index: None,
        };

        observer.on_streaming_token(event);

        let ui_event = rx.try_recv().expect("should receive Token");
        match ui_event {
            UiEvent::Token {
                content,
                is_thinking,
            } => {
                assert_eq!(content, "let me consider");
                assert!(is_thinking);
            }
            other => panic!("expected Token, got {:?}", other),
        }
    }

    #[test]
    fn streaming_tool_call_events_are_ignored() {
        let (mut rx, observer) = setup();

        let event = StreamingTokenEvent {
            event_type: "tool_call_start".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: Some("search".to_owned()),
                    args: None,
                    id: Some("call_5".to_owned()),
                }),
            },
            index: Some(0),
        };

        observer.on_streaming_token(event);

        assert!(
            rx.try_recv().is_err(),
            "tool_call_start should not produce a UiEvent"
        );
    }

    #[test]
    fn tool_done_without_prior_start_yields_zero_duration() {
        let (mut rx, observer) = setup();

        // Complete a tool that was never started (no start time recorded)
        observer.on_tool(ConversationTurnToolEvent::completed(
            "orphan_call",
            "read_file",
            Some("file contents".to_owned()),
        ));

        let event = rx.try_recv().expect("should receive ToolDone");
        match event {
            UiEvent::ToolDone {
                tool_id,
                duration_ms,
                ..
            } => {
                assert_eq!(tool_id, "orphan_call");
                assert_eq!(duration_ms, 0);
            }
            other => panic!("expected ToolDone, got {:?}", other),
        }
    }

    #[test]
    fn denied_tool_sends_tool_done_failure() {
        let (mut rx, observer) = setup();

        observer.on_tool(ConversationTurnToolEvent::denied(
            "call_6",
            "shell",
            "user denied",
        ));

        let event = rx.try_recv().expect("should receive ToolDone");
        match event {
            UiEvent::ToolDone {
                success, output, ..
            } => {
                assert!(!success);
                assert_eq!(output, "user denied");
            }
            other => panic!("expected ToolDone, got {:?}", other),
        }
    }

    #[test]
    fn completed_phase_without_estimated_tokens_defaults_to_zero() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::completed(5, None));

        let _ = rx.try_recv(); // PhaseChange

        let done_event = rx.try_recv().expect("should receive ResponseDone");
        match done_event {
            UiEvent::ResponseDone { input_tokens, .. } => {
                assert_eq!(input_tokens, 0);
            }
            other => panic!("expected ResponseDone, got {:?}", other),
        }
    }
}
