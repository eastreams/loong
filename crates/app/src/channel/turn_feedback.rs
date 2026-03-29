use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use crate::conversation::ConversationTurnObserver;
use crate::conversation::ConversationTurnObserverHandle;
use crate::conversation::ConversationTurnPhase;
use crate::conversation::ConversationTurnPhaseEvent;
use crate::conversation::ConversationTurnToolEvent;
use crate::conversation::ConversationTurnToolState;

const CHANNEL_TURN_TRACE_MAX_LINES: usize = 4;
const CHANNEL_TURN_TRACE_MAX_DETAIL_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChannelTurnTraceMode {
    #[default]
    Off,
    Significant,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelTurnFeedbackPolicy {
    trace_mode: ChannelTurnTraceMode,
}

impl ChannelTurnFeedbackPolicy {
    pub const fn disabled() -> Self {
        Self {
            trace_mode: ChannelTurnTraceMode::Off,
        }
    }

    pub const fn final_trace_significant() -> Self {
        Self {
            trace_mode: ChannelTurnTraceMode::Significant,
        }
    }

    pub const fn requires_observer(self) -> bool {
        !matches!(self.trace_mode, ChannelTurnTraceMode::Off)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChannelTurnFeedbackState {
    latest_phase: Option<ConversationTurnPhase>,
    latest_tool_events: Vec<ConversationTurnToolEvent>,
}

#[derive(Debug, Default)]
struct ChannelTurnFeedbackObserver {
    state: StdMutex<ChannelTurnFeedbackState>,
}

#[derive(Debug, Clone)]
pub(super) struct ChannelTurnFeedbackCapture {
    policy: ChannelTurnFeedbackPolicy,
    observer: Option<Arc<ChannelTurnFeedbackObserver>>,
}

impl ChannelTurnFeedbackCapture {
    pub(super) fn new(policy: ChannelTurnFeedbackPolicy) -> Self {
        let observer = if policy.requires_observer() {
            Some(Arc::new(ChannelTurnFeedbackObserver::default()))
        } else {
            None
        };

        Self { policy, observer }
    }

    pub(super) fn observer_handle(&self) -> Option<ConversationTurnObserverHandle> {
        let observer = self.observer.as_ref()?;
        let handle: ConversationTurnObserverHandle = observer.clone();
        Some(handle)
    }

    pub(super) fn render_reply(&self, reply: String) -> String {
        let Some(observer) = self.observer.as_ref() else {
            return reply;
        };

        let Some(trace) = observer.render_trace(self.policy) else {
            return reply;
        };

        format_channel_turn_reply_with_trace(reply, trace)
    }
}

impl ChannelTurnFeedbackObserver {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, ChannelTurnFeedbackState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        }
    }

    fn record_phase(&self, event: ConversationTurnPhaseEvent) {
        let mut state = self.lock_state();
        state.latest_phase = Some(event.phase);
    }

    fn record_tool(&self, event: ConversationTurnToolEvent) {
        let mut state = self.lock_state();
        upsert_channel_turn_tool_event(&mut state.latest_tool_events, event);
    }

    fn render_trace(&self, policy: ChannelTurnFeedbackPolicy) -> Option<String> {
        let state = self.lock_state();
        render_channel_turn_trace(&state, policy)
    }
}

impl ConversationTurnObserver for ChannelTurnFeedbackObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        self.record_phase(event);
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        self.record_tool(event);
    }
}

fn upsert_channel_turn_tool_event(
    latest_tool_events: &mut Vec<ConversationTurnToolEvent>,
    event: ConversationTurnToolEvent,
) {
    let position = latest_tool_events
        .iter()
        .position(|existing| existing.tool_call_id == event.tool_call_id);

    match position {
        Some(index) => {
            if let Some(existing_event) = latest_tool_events.get_mut(index) {
                *existing_event = event;
            }
        }
        None => {
            latest_tool_events.push(event);
        }
    }
}

fn render_channel_turn_trace(
    state: &ChannelTurnFeedbackState,
    policy: ChannelTurnFeedbackPolicy,
) -> Option<String> {
    match policy.trace_mode {
        ChannelTurnTraceMode::Off => None,
        ChannelTurnTraceMode::Significant => {
            render_significant_channel_turn_trace(state.latest_phase, &state.latest_tool_events)
        }
    }
}

fn render_significant_channel_turn_trace(
    latest_phase: Option<ConversationTurnPhase>,
    latest_tool_events: &[ConversationTurnToolEvent],
) -> Option<String> {
    let mut trace_lines = latest_tool_events
        .iter()
        .filter_map(format_significant_channel_turn_tool_event)
        .collect::<Vec<_>>();

    if trace_lines.is_empty() && latest_phase == Some(ConversationTurnPhase::Failed) {
        trace_lines.push("- turn failed before a stable reply was produced".to_owned());
    }

    if trace_lines.is_empty() {
        return None;
    }

    if trace_lines.len() > CHANNEL_TURN_TRACE_MAX_LINES {
        let visible_line_limit = CHANNEL_TURN_TRACE_MAX_LINES.saturating_sub(1);
        let remaining_count = trace_lines.len().saturating_sub(visible_line_limit);
        trace_lines.truncate(visible_line_limit);
        let noun = if remaining_count == 1 {
            "event"
        } else {
            "events"
        };
        let summary_line = format!("- ... and {remaining_count} more {noun}");
        trace_lines.push(summary_line);
    }

    let trace_body = trace_lines.join("\n");
    Some(format!("execution trace:\n{trace_body}"))
}

fn format_significant_channel_turn_tool_event(event: &ConversationTurnToolEvent) -> Option<String> {
    let detail = event
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let is_significant =
        !matches!(event.state, ConversationTurnToolState::Completed) || detail.is_some();
    if !is_significant {
        return None;
    }

    let state = event.state.as_str();
    match detail {
        Some(detail) => {
            let summarized_detail = summarize_channel_turn_trace_detail(detail);
            Some(format!(
                "- {} {}: {}",
                event.tool_name, state, summarized_detail
            ))
        }
        None => Some(format!("- {} {}", event.tool_name, state)),
    }
}

fn summarize_channel_turn_trace_detail(detail: &str) -> String {
    let trimmed = detail.trim();
    let total_chars = trimmed.chars().count();
    if total_chars <= CHANNEL_TURN_TRACE_MAX_DETAIL_CHARS {
        return trimmed.to_owned();
    }

    let summarized = trimmed
        .chars()
        .take(CHANNEL_TURN_TRACE_MAX_DETAIL_CHARS)
        .collect::<String>();
    format!("{summarized}...")
}

fn format_channel_turn_reply_with_trace(reply: String, trace: String) -> String {
    let trimmed_reply = reply.trim();
    if trimmed_reply.is_empty() {
        return trace;
    }

    format!("{reply}\n\n{trace}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationTurnToolEvent;

    #[test]
    fn channel_turn_feedback_capture_renders_significant_final_trace() {
        let capture =
            ChannelTurnFeedbackCapture::new(ChannelTurnFeedbackPolicy::final_trace_significant());
        let observer = capture
            .observer_handle()
            .expect("significant trace policy should attach an observer");

        observer.on_tool(ConversationTurnToolEvent::running("call-1", "tool.search"));
        observer.on_tool(ConversationTurnToolEvent::completed(
            "call-1",
            "tool.search",
            Some("returned 0 results".to_owned()),
        ));
        observer.on_tool(ConversationTurnToolEvent::failed(
            "call-2",
            "web.fetch",
            "missing network egress capability",
        ));

        let reply = capture.render_reply("final reply".to_owned());

        assert!(reply.contains("final reply"));
        assert!(reply.contains("execution trace:"));
        assert!(reply.contains("- tool.search completed: returned 0 results"));
        assert!(reply.contains("- web.fetch failed: missing network egress capability"));
        assert!(
            !reply.contains("tool.search running"),
            "the capture should keep only the latest event per tool call"
        );
    }

    #[test]
    fn channel_turn_feedback_capture_skips_trace_when_policy_is_disabled() {
        let capture = ChannelTurnFeedbackCapture::new(ChannelTurnFeedbackPolicy::disabled());
        let reply = capture.render_reply("{\"status\":\"ok\"}".to_owned());

        assert_eq!(reply, "{\"status\":\"ok\"}");
        assert!(capture.observer_handle().is_none());
    }

    #[test]
    fn channel_turn_feedback_capture_ignores_non_significant_completed_tools() {
        let capture =
            ChannelTurnFeedbackCapture::new(ChannelTurnFeedbackPolicy::final_trace_significant());
        let observer = capture
            .observer_handle()
            .expect("significant trace policy should attach an observer");

        observer.on_tool(ConversationTurnToolEvent::completed(
            "call-1",
            "shell.exec",
            None,
        ));

        let reply = capture.render_reply("final reply".to_owned());

        assert_eq!(reply, "final reply");
    }
}
