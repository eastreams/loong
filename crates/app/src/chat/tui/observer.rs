use std::collections::{HashMap, HashSet};
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
    stream_tool_call_ids: HashMap<usize, String>,
    announced_tool_call_ids: HashSet<String>,
    latest_phase: String,
}

impl ObserverState {
    fn new() -> Self {
        Self {
            tool_start_times: HashMap::new(),
            stream_tool_call_ids: HashMap::new(),
            announced_tool_call_ids: HashSet::new(),
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

fn remove_tool_call_tracking(state: &mut ObserverState, tool_call_id: &str) {
    state.announced_tool_call_ids.remove(tool_call_id);
    state
        .stream_tool_call_ids
        .retain(|_, tracked_tool_call_id| tracked_tool_call_id != tool_call_id);
}

fn format_phase_label(phase: ConversationTurnPhase) -> String {
    let raw_phase = phase.as_str();
    let mut words = Vec::new();

    for (index, segment) in raw_phase.split('_').enumerate() {
        if segment.is_empty() {
            continue;
        }

        let word = if index == 0 {
            let mut chars = segment.chars();
            let Some(first_char) = chars.next() else {
                continue;
            };

            let first_upper = first_char.to_ascii_uppercase();
            let remaining = chars.as_str();
            let mut capitalized = String::new();
            capitalized.push(first_upper);
            capitalized.push_str(remaining);
            capitalized
        } else {
            segment.to_owned()
        };

        words.push(word);
    }

    words.join(" ")
}

fn format_lane_label(lane: crate::conversation::ExecutionLane) -> &'static str {
    match lane {
        crate::conversation::ExecutionLane::Fast => "fast",
        crate::conversation::ExecutionLane::Safe => "safe",
    }
}

fn format_compact_count(value: usize) -> String {
    if value < 1_000 {
        return value.to_string();
    }

    let whole_thousands = value / 1_000;
    let fractional_hundreds = (value % 1_000) / 100;

    if whole_thousands >= 10 || fractional_hundreds == 0 {
        return format!("{whole_thousands}k");
    }

    format!("{whole_thousands}.{fractional_hundreds}k")
}

fn summarize_phase_action(event: &ConversationTurnPhaseEvent) -> String {
    let mut parts = Vec::new();

    if let Some(lane) = event.lane {
        let lane_label = format_lane_label(lane);
        let lane_summary = format!("{lane_label} lane");
        parts.push(lane_summary);
    }

    if event.tool_call_count > 0 {
        let tool_label = if event.tool_call_count == 1 {
            "tool"
        } else {
            "tools"
        };
        let tool_summary = format!("{} {tool_label}", event.tool_call_count);
        parts.push(tool_summary);
    }

    if let Some(message_count) = event.message_count {
        let message_label = if message_count == 1 {
            "message"
        } else {
            "messages"
        };
        let message_summary = format!("{message_count} {message_label}");
        parts.push(message_summary);
    }

    if let Some(estimated_tokens) = event.estimated_tokens {
        let compact_tokens = format_compact_count(estimated_tokens);
        let token_summary = format!("est. {compact_tokens} tok");
        parts.push(token_summary);
    }

    parts.join(" | ")
}

impl ConversationTurnObserver for TuiObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        let phase_str = format_phase_label(event.phase);
        let iteration = event.provider_round.unwrap_or(0) as u32;
        let action = summarize_phase_action(&event);

        {
            let mut state = self.lock_state();
            state.latest_phase = phase_str.clone();
            if matches!(
                event.phase,
                ConversationTurnPhase::RequestingProvider
                    | ConversationTurnPhase::RequestingFollowupProvider
            ) {
                state.stream_tool_call_ids.clear();
            }
        }

        let _ = self.tx.send(UiEvent::PhaseChange {
            phase: phase_str,
            iteration,
            action,
        });

        if event.phase == ConversationTurnPhase::Completed {
            let input_tokens = event
                .actual_input_tokens
                .unwrap_or_else(|| event.estimated_tokens.unwrap_or(0) as u32);
            let output_tokens = event.actual_output_tokens.unwrap_or(0);
            let _ = self.tx.send(UiEvent::ResponseDone {
                input_tokens,
                output_tokens,
            });
        }
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        match event.state {
            ConversationTurnToolState::Running => {
                let tool_call_id = event.tool_call_id;
                let tool_name = event.tool_name;
                let args_preview = event.detail.unwrap_or_default();
                let should_emit_tool_start = {
                    let mut state = self.lock_state();
                    state
                        .tool_start_times
                        .entry(tool_call_id.clone())
                        .or_insert_with(Instant::now);
                    state.announced_tool_call_ids.insert(tool_call_id.clone())
                };

                if should_emit_tool_start {
                    let _ = self.tx.send(UiEvent::ToolStart {
                        tool_id: tool_call_id,
                        tool_name,
                        args_preview,
                    });
                    return;
                }

                if !args_preview.is_empty() {
                    let _ = self.tx.send(UiEvent::ToolArgsDelta {
                        tool_id: tool_call_id,
                        chunk: args_preview,
                    });
                }
            }

            ConversationTurnToolState::Completed
            | ConversationTurnToolState::Failed
            | ConversationTurnToolState::Interrupted => {
                let tool_call_id = event.tool_call_id;
                let output = event.detail.unwrap_or_default();
                let duration_ms = {
                    let mut state = self.lock_state();
                    let duration = state
                        .tool_start_times
                        .remove(&tool_call_id)
                        .map(|start| start.elapsed().as_millis().min(u32::MAX as u128) as u32)
                        .unwrap_or(0);
                    remove_tool_call_tracking(&mut state, tool_call_id.as_str());
                    duration
                };

                let success = event.state == ConversationTurnToolState::Completed;

                let _ = self.tx.send(UiEvent::ToolDone {
                    tool_id: tool_call_id,
                    success,
                    output,
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
                let tool_call_id = event.tool_call_id;
                let output = event.detail.unwrap_or_else(|| "denied".to_owned());
                let duration_ms = {
                    let mut state = self.lock_state();
                    let duration = state
                        .tool_start_times
                        .remove(&tool_call_id)
                        .map(|start| start.elapsed().as_millis().min(u32::MAX as u128) as u32)
                        .unwrap_or(0);
                    remove_tool_call_tracking(&mut state, tool_call_id.as_str());
                    duration
                };

                let _ = self.tx.send(UiEvent::ToolDone {
                    tool_id: tool_call_id,
                    success: false,
                    output,
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
            "tool_call_start" => {
                let stream_index = match event.index {
                    Some(stream_index) => stream_index,
                    None => return,
                };
                let tool_call = match event.delta.tool_call {
                    Some(tool_call) => tool_call,
                    None => return,
                };
                let tool_call_id = match tool_call.id {
                    Some(tool_call_id) => tool_call_id,
                    None => return,
                };
                let tool_name = match tool_call.name {
                    Some(tool_name) => tool_name,
                    None => return,
                };
                let should_emit_tool_start = {
                    let mut state = self.lock_state();
                    state
                        .tool_start_times
                        .entry(tool_call_id.clone())
                        .or_insert_with(Instant::now);
                    state
                        .stream_tool_call_ids
                        .insert(stream_index, tool_call_id.clone());
                    state.announced_tool_call_ids.insert(tool_call_id.clone())
                };

                if should_emit_tool_start {
                    let _ = self.tx.send(UiEvent::ToolStart {
                        tool_id: tool_call_id,
                        tool_name,
                        args_preview: String::new(),
                    });
                }
            }
            "tool_call_input_delta" => {
                let stream_index = match event.index {
                    Some(stream_index) => stream_index,
                    None => return,
                };
                let tool_call = match event.delta.tool_call {
                    Some(tool_call) => tool_call,
                    None => return,
                };
                let chunk = match tool_call.args {
                    Some(chunk) => chunk,
                    None => return,
                };
                let tool_call_id = {
                    let state = self.lock_state();
                    state.stream_tool_call_ids.get(&stream_index).cloned()
                };
                let Some(tool_call_id) = tool_call_id else {
                    return;
                };

                let _ = self.tx.send(UiEvent::ToolArgsDelta {
                    tool_id: tool_call_id,
                    chunk,
                });
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
                assert_eq!(phase, "Requesting provider");
                assert_eq!(iteration, 2);
                assert_eq!(action, "10 messages | est. 500 tok");
            }
            other => panic!("expected PhaseChange, got {:?}", other),
        }

        assert!(rx.try_recv().is_err(), "no extra events expected");
    }

    #[test]
    fn running_tools_phase_sends_lane_and_tool_summary() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::running_tools(
            3,
            crate::conversation::ExecutionLane::Safe,
            2,
        ));

        let event = rx.try_recv().expect("should receive PhaseChange");
        match event {
            UiEvent::PhaseChange {
                phase,
                iteration,
                action,
            } => {
                assert_eq!(phase, "Running tools");
                assert_eq!(iteration, 3);
                assert_eq!(action, "safe lane | 2 tools");
            }
            other => panic!("expected PhaseChange, got {:?}", other),
        }
    }

    #[test]
    fn completed_phase_sends_response_done_with_actual_tokens() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::completed(
            12,
            Some(1500),
            Some(1200),
            Some(350),
        ));

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
                assert_eq!(input_tokens, 1200);
                assert_eq!(output_tokens, 350);
            }
            other => panic!("expected ResponseDone, got {:?}", other),
        }
    }

    #[test]
    fn completed_phase_falls_back_to_estimated_when_no_actual() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::completed(
            12,
            Some(1500),
            None,
            None,
        ));

        let _ = rx.try_recv(); // consume PhaseChange

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
    fn running_tools_phase_sends_lane_and_tool_summary_action() {
        let (mut rx, observer) = setup();

        observer.on_phase(ConversationTurnPhaseEvent::running_tools(
            3,
            crate::conversation::ExecutionLane::Safe,
            2,
        ));

        let event = rx.try_recv().expect("should receive PhaseChange");
        match event {
            UiEvent::PhaseChange {
                phase,
                iteration,
                action,
            } => {
                assert_eq!(phase, "Running tools");
                assert_eq!(iteration, 3);
                assert_eq!(action, "safe lane | 2 tools");
            }
            other => panic!("expected PhaseChange, got {:?}", other),
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
    fn streaming_tool_call_start_sends_tool_start() {
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

        let ui_event = rx.try_recv().expect("should receive ToolStart");
        match ui_event {
            UiEvent::ToolStart {
                tool_id,
                tool_name,
                args_preview,
            } => {
                assert_eq!(tool_id, "call_5");
                assert_eq!(tool_name, "search");
                assert!(args_preview.is_empty());
            }
            other => panic!("expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn streaming_tool_call_start_then_running_emits_single_tool_start() {
        let (mut rx, observer) = setup();

        let start_event = StreamingTokenEvent {
            event_type: "tool_call_start".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: Some("search".to_owned()),
                    args: None,
                    id: Some("call_7".to_owned()),
                }),
            },
            index: Some(0),
        };

        observer.on_streaming_token(start_event);
        observer.on_tool(ConversationTurnToolEvent::running("call_7", "search"));

        let first_event = rx.try_recv().expect("should receive ToolStart");
        match first_event {
            UiEvent::ToolStart {
                tool_id, tool_name, ..
            } => {
                assert_eq!(tool_id, "call_7");
                assert_eq!(tool_name, "search");
            }
            other => panic!("expected ToolStart, got {:?}", other),
        }

        assert!(
            rx.try_recv().is_err(),
            "running event should not emit a duplicate ToolStart after streaming start"
        );
    }

    #[test]
    fn streaming_tool_call_input_delta_emits_tool_args_delta() {
        let (mut rx, observer) = setup();

        observer.on_streaming_token(StreamingTokenEvent {
            event_type: "tool_call_start".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: Some("file.write".to_owned()),
                    args: None,
                    id: Some("call_9".to_owned()),
                }),
            },
            index: Some(2),
        });

        let _ = rx.try_recv().expect("should receive ToolStart");

        observer.on_streaming_token(StreamingTokenEvent {
            event_type: "tool_call_input_delta".to_owned(),
            delta: crate::acp::TokenDelta {
                text: None,
                tool_call: Some(crate::acp::ToolCallDelta {
                    name: None,
                    args: Some("{\"path\":\"src/main.rs\"}".to_owned()),
                    id: None,
                }),
            },
            index: Some(2),
        });

        let ui_event = rx.try_recv().expect("should receive ToolArgsDelta");
        match ui_event {
            UiEvent::ToolArgsDelta { tool_id, chunk } => {
                assert_eq!(tool_id, "call_9");
                assert_eq!(chunk, "{\"path\":\"src/main.rs\"}");
            }
            other => panic!("expected ToolArgsDelta, got {:?}", other),
        }
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

        observer.on_phase(ConversationTurnPhaseEvent::completed(5, None, None, None));

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
