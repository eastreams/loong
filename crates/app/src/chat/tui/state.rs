use std::time::Instant;

use super::dialog::ClarifyDialog;
use super::focus::FocusTarget;
use super::message::{Message, MessagePart, ToolStatus};

const SPINNER_INTERVAL_MS: u128 = 80;
const DOTS_INTERVAL_MS: u128 = 300;
const SPINNER_FRAMES: usize = 10;
const DOTS_FRAMES: usize = 4;

#[derive(Debug, Clone)]
pub(super) struct Pane {
    pub(super) messages: Vec<Message>,
    pub(super) scroll_offset: u16,
    pub(super) session_id: String,
    pub(super) model: String,
    pub(super) input_tokens: u32,
    pub(super) output_tokens: u32,
    pub(super) context_length: u32,
    pub(super) agent_running: bool,
    pub(super) loop_state: String,
    pub(super) loop_iteration: u32,
    pub(super) streaming_active: bool,
    pub(super) spinner_frame: usize,
    pub(super) dots_frame: usize,
    pub(super) last_spinner_tick: Instant,
    pub(super) status_message: Option<(String, Instant)>,
    pub(super) clarify_dialog: Option<ClarifyDialog>,
}

impl Pane {
    pub(super) fn new(session_id: &str) -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            session_id: session_id.to_string(),
            model: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            context_length: 0,
            agent_running: false,
            loop_state: String::new(),
            loop_iteration: 0,
            streaming_active: false,
            spinner_frame: 0,
            dots_frame: 0,
            last_spinner_tick: Instant::now(),
            status_message: None,
            clarify_dialog: None,
        }
    }

    pub(super) fn append_token(&mut self, content: &str, is_thinking: bool) {
        self.streaming_active = true;
        self.ensure_assistant_message();
        let msg = match self.messages.last_mut() {
            Some(m) => m,
            None => return,
        };
        let extend_existing = match msg.parts.last() {
            Some(MessagePart::ThinkBlock(_)) if is_thinking => true,
            Some(MessagePart::Text(_)) if !is_thinking => true,
            _ => false,
        };
        if extend_existing {
            match msg.parts.last_mut() {
                Some(MessagePart::ThinkBlock(text)) | Some(MessagePart::Text(text)) => {
                    text.push_str(content);
                }
                _ => {}
            }
        } else {
            let part = if is_thinking {
                MessagePart::ThinkBlock(content.to_string())
            } else {
                MessagePart::Text(content.to_string())
            };
            msg.parts.push(part);
        }
    }

    /// Adds a `ToolCall` part with `ToolStatus::Running` to the last assistant
    /// message.
    pub(super) fn start_tool_call(&mut self, tool_id: &str, tool_name: &str, args_preview: &str) {
        self.ensure_assistant_message();
        if let Some(msg) = self.messages.last_mut() {
            msg.parts.push(MessagePart::ToolCall {
                tool_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
                args_preview: args_preview.to_string(),
                status: ToolStatus::Running {
                    started: Instant::now(),
                },
            });
        }
    }

    /// Finds the matching tool call by `tool_id` and transitions it to
    /// `ToolStatus::Done`. Output is truncated to 80 chars for the preview.
    pub(super) fn complete_tool_call(
        &mut self,
        tool_id: &str,
        success: bool,
        output: &str,
        duration_ms: u32,
    ) {
        let truncated = truncate_preview(output, 80);
        for msg in self.messages.iter_mut().rev() {
            for part in &mut msg.parts {
                if let MessagePart::ToolCall {
                    tool_id: id,
                    status,
                    ..
                } = part
                    && id == tool_id
                {
                    *status = ToolStatus::Done {
                        success,
                        output: truncated,
                        duration_ms,
                    };
                    return;
                }
            }
        }
    }

    pub(super) fn finalize_response(&mut self, input_tokens: u32, output_tokens: u32) {
        self.streaming_active = false;
        self.input_tokens = self.input_tokens.saturating_add(input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(output_tokens);
        self.agent_running = false;
    }

    pub(super) fn add_user_message(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    pub(super) fn add_system_message(&mut self, text: &str) {
        self.messages.push(Message::system(text));
    }

    pub(super) fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Returns context usage as a fraction in `[0.0, 1.0]`.
    /// Returns `0.0` when `context_length` is zero to avoid division by zero.
    pub(super) fn context_percent(&self) -> f32 {
        if self.context_length == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let pct = self.total_tokens() as f32 / self.context_length as f32;
        pct
    }

    /// Advances spinner and dots frames based on elapsed time since the last
    /// tick call.
    pub(super) fn tick_spinner(&mut self) {
        let elapsed = self.last_spinner_tick.elapsed().as_millis();
        if elapsed >= SPINNER_INTERVAL_MS {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES;
            if elapsed >= DOTS_INTERVAL_MS {
                self.dots_frame = (self.dots_frame + 1) % DOTS_FRAMES;
            }
            self.last_spinner_tick = Instant::now();
        }
    }

    pub(super) fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    // -- private helpers --

    fn ensure_assistant_message(&mut self) {
        let needs_new = self
            .messages
            .last()
            .is_none_or(|m| m.role != super::message::Role::Assistant);
        if needs_new {
            self.messages.push(Message::assistant());
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Shell {
    pub(super) pane: Pane,
    pub(super) running: bool,
    pub(super) show_thinking: bool,
    pub(super) show_help: bool,
}

impl Shell {
    pub(super) fn new(session_id: &str) -> Self {
        Self {
            pane: Pane::new(session_id),
            running: true,
            show_thinking: true,
            show_help: false,
        }
    }
}

// ---------------------------------------------------------------------------
// UiState: top-level TUI state combining pane with focus and drawer
// ---------------------------------------------------------------------------

/// Top-level TUI state combining pane state with focus and drawer visibility.
/// Used as the single source of truth for the render loop.
#[derive(Debug, Clone)]
pub(crate) struct UiState {
    pub(crate) session_id: String,
    pub(super) pane: Pane,
    pub(crate) focus_target: FocusTarget,
    pub(crate) drawer: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            pane: Pane::new("default"),
            focus_target: FocusTarget::Composer,
            drawer: None,
        }
    }
}

impl UiState {
    pub(crate) fn with_session_id(session_id: impl Into<String>) -> Self {
        let id: String = session_id.into();
        Self {
            session_id: id.clone(),
            pane: Pane::new(&id),
            ..Self::default()
        }
    }
}

/// Truncates a string to at most `max_chars` characters.
fn truncate_preview(s: &str, max_chars: usize) -> String {
    let end = s.char_indices().nth(max_chars).map_or(s.len(), |(i, _)| i);
    s.get(..end).unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_token_tracking() {
        let mut pane = Pane::new("sess-1");
        assert_eq!(pane.total_tokens(), 0);

        pane.finalize_response(100, 50);
        assert_eq!(pane.input_tokens, 100);
        assert_eq!(pane.output_tokens, 50);
        assert_eq!(pane.total_tokens(), 150);

        pane.finalize_response(200, 100);
        assert_eq!(pane.total_tokens(), 450);
    }

    #[test]
    fn context_percent_zero_length() {
        let pane = Pane::new("sess-1");
        assert!((pane.context_percent() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn context_percent_calculation() {
        let mut pane = Pane::new("sess-1");
        pane.context_length = 1000;
        pane.input_tokens = 250;
        pane.output_tokens = 250;
        let pct = pane.context_percent();
        assert!((pct - 0.5).abs() < 0.001);
    }

    #[test]
    fn spinner_tick_advances() {
        let mut pane = Pane::new("sess-1");
        let initial_frame = pane.spinner_frame;
        // Force the tick interval to have elapsed
        pane.last_spinner_tick = Instant::now() - std::time::Duration::from_millis(100);
        pane.tick_spinner();
        assert_ne!(pane.spinner_frame, initial_frame);
    }

    #[test]
    fn append_and_flush_streaming() {
        let mut pane = Pane::new("sess-1");
        pane.append_token("hello ", false);
        pane.append_token("world", false);
        assert!(pane.streaming_active);
        assert_eq!(pane.messages.len(), 1);
        assert_eq!(pane.messages[0].parts.len(), 1);
        match &pane.messages[0].parts[0] {
            MessagePart::Text(text) => assert_eq!(text, "hello world"),
            other @ MessagePart::ThinkBlock(_) | other @ MessagePart::ToolCall { .. } => {
                panic!("expected Text, got {:?}", other)
            }
        }
    }

    #[test]
    fn thinking_toggle_creates_separate_parts() {
        let mut pane = Pane::new("sess-1");
        pane.append_token("thought", true);
        pane.append_token("visible", false);
        assert_eq!(pane.messages.len(), 1);
        let parts = &pane.messages[0].parts;
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::ThinkBlock(t) if t == "thought"));
        assert!(matches!(&parts[1], MessagePart::Text(t) if t == "visible"));
    }

    #[test]
    fn tool_call_lifecycle() {
        let mut pane = Pane::new("sess-1");
        pane.start_tool_call("t1", "read_file", "path=/foo");
        assert_eq!(pane.messages.len(), 1);

        pane.complete_tool_call("t1", true, "file contents here", 42);
        if let Some(msg) = pane.messages.last() {
            if let Some(MessagePart::ToolCall { status, .. }) = msg.parts.last() {
                match status {
                    ToolStatus::Done {
                        success,
                        duration_ms,
                        ..
                    } => {
                        assert!(success);
                        assert_eq!(*duration_ms, 42);
                    }
                    ToolStatus::Running { .. } => {
                        panic!("expected Done status");
                    }
                }
            } else {
                panic!("expected ToolCall part");
            }
        }
    }

    #[test]
    fn set_status_records_instant() {
        let mut pane = Pane::new("sess-1");
        assert!(pane.status_message.is_none());
        pane.set_status("connecting...".into());
        assert!(pane.status_message.is_some());
        assert_eq!(
            pane.status_message.as_ref().map(|(m, _)| m.as_str()),
            Some("connecting...")
        );
    }

    #[test]
    fn shell_defaults() {
        let shell = Shell::new("s1");
        assert!(shell.running);
        assert!(shell.show_thinking);
        assert!(!shell.show_help);
        assert_eq!(shell.pane.session_id, "s1");
    }
}
