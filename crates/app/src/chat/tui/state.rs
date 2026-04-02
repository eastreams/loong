use std::time::Instant;

use super::dialog::ClarifyDialog;
use super::focus::FocusStack;
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
    pub(super) input_hint_override: Option<String>,
    /// Depth-1 staged message queue: holds the next user message to submit
    /// once the current agent turn completes.
    pub(super) staged_message: Option<String>,
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
            input_hint_override: None,
            staged_message: None,
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

    pub(super) fn show_surface_lines(&mut self, lines: &[String]) {
        let content = lines.join("\n");
        self.messages.clear();
        self.messages.push(Message::surface(content));
    }

    pub(super) fn clear_input_hint_override(&mut self) {
        self.input_hint_override = None;
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
    pub(super) focus: FocusStack,
    pub(super) dirty: bool,
}

impl Shell {
    pub(super) fn new(session_id: &str) -> Self {
        Self {
            pane: Pane::new(session_id),
            running: true,
            show_thinking: true,
            focus: FocusStack::new(),
            dirty: true,
        }
    }
}

// ---------------------------------------------------------------------------
// UiState: top-level TUI state combining pane with focus and drawer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct UiState {
    pub(crate) session_id: String,
    pub(super) pane: Pane,
    pub(crate) focus: FocusStack,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            pane: Pane::new("default"),
            focus: FocusStack::new(),
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

/// Infer the context window size (in tokens) from a model name string.
///
/// Uses well-known model-name prefixes/substrings to return the advertised
/// context window.  Falls back to `0` for unrecognised models so that the
/// percentage display degrades gracefully (shows 0%).
pub(super) fn context_length_for_model(model: &str) -> u32 {
    let m = model.to_ascii_lowercase();

    // --- Anthropic Claude ---------------------------------------------------
    // Claude 3.x / 4.x: 200k
    // Claude 2.x / Instant: 100k
    if m.contains("claude") {
        if m.contains("claude-3") || m.contains("claude-4") {
            return 200_000;
        }
        return 100_000;
    }

    // --- OpenAI reasoning models (o1, o3, o4-mini) --------------------------
    if m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        return 200_000;
    }

    // --- OpenAI GPT ---------------------------------------------------------
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") || m.contains("gpt-4-1") {
        return 128_000;
    }
    if m.contains("gpt-4") {
        return 8_192;
    }
    if m.contains("gpt-3.5") {
        return 16_385;
    }

    // --- Google Gemini (1.5 / 2.x) -----------------------------------------
    if m.contains("gemini") {
        return 1_048_576;
    }

    // --- Mistral / Mixtral --------------------------------------------------
    if m.contains("mistral") || m.contains("mixtral") {
        return 32_768;
    }

    // --- DeepSeek -----------------------------------------------------------
    if m.contains("deepseek") {
        return 64_000;
    }

    // Unknown model — 0 keeps the percentage at 0%.
    0
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
        assert!(shell.dirty);
        assert_eq!(shell.focus.top(), super::super::focus::FocusLayer::Composer);
        assert_eq!(shell.pane.session_id, "s1");
    }

    #[test]
    fn staged_message_defaults_to_none() {
        let pane = Pane::new("sess-1");
        assert!(pane.staged_message.is_none());
    }

    #[test]
    fn staged_message_last_wins() {
        let mut pane = Pane::new("sess-1");
        pane.staged_message = Some("first".to_string());
        pane.staged_message = Some("second".to_string());
        assert_eq!(pane.staged_message.as_deref(), Some("second"));
    }

    #[test]
    fn staged_message_take_clears() {
        let mut pane = Pane::new("sess-1");
        pane.staged_message = Some("queued".to_string());
        let taken = pane.staged_message.take();
        assert_eq!(taken.as_deref(), Some("queued"));
        assert!(pane.staged_message.is_none());
    }

    // -- context_length_for_model tests --

    #[test]
    fn context_length_claude_3_models() {
        assert_eq!(context_length_for_model("claude-3-opus-20240229"), 200_000);
        assert_eq!(context_length_for_model("claude-3.5-sonnet"), 200_000);
        assert_eq!(context_length_for_model("claude-3-haiku"), 200_000);
    }

    #[test]
    fn context_length_claude_4_models() {
        assert_eq!(context_length_for_model("claude-4-opus"), 200_000);
    }

    #[test]
    fn context_length_claude_2_models() {
        assert_eq!(context_length_for_model("claude-2.1"), 100_000);
        assert_eq!(context_length_for_model("claude-instant-1.2"), 100_000);
    }

    #[test]
    fn context_length_openai_gpt4o() {
        assert_eq!(context_length_for_model("gpt-4o"), 128_000);
        assert_eq!(context_length_for_model("gpt-4o-mini"), 128_000);
        assert_eq!(context_length_for_model("gpt-4-turbo"), 128_000);
    }

    #[test]
    fn context_length_openai_gpt4_base() {
        assert_eq!(context_length_for_model("gpt-4"), 8_192);
    }

    #[test]
    fn context_length_openai_reasoning() {
        assert_eq!(context_length_for_model("o1-preview"), 200_000);
        assert_eq!(context_length_for_model("o3-mini"), 200_000);
    }

    #[test]
    fn context_length_gemini() {
        assert_eq!(context_length_for_model("gemini-1.5-pro"), 1_048_576);
        assert_eq!(context_length_for_model("gemini-2.0-flash"), 1_048_576);
    }

    #[test]
    fn context_length_unknown_model() {
        assert_eq!(context_length_for_model("some-custom-model"), 0);
        assert_eq!(context_length_for_model("auto"), 0);
        assert_eq!(context_length_for_model(""), 0);
    }
}
