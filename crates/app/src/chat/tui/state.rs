use std::time::Instant;

use super::dialog::ClarifyDialog;
use super::focus::FocusStack;
use super::message::{Message, MessagePart, ToolStatus};

const SPINNER_INTERVAL_MS: u128 = 80;
const DOTS_INTERVAL_MS: u128 = 300;
const SPINNER_FRAMES: usize = 10;
const DOTS_FRAMES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolInspectorState {
    pub(super) selected_tool_id: String,
    pub(super) scroll_offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TranscriptReviewState {
    pub(super) cursor_line: usize,
    pub(super) anchor_line: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ToolCallRecord<'a> {
    pub(super) tool_id: &'a str,
    pub(super) tool_name: &'a str,
    pub(super) args_preview: &'a str,
    pub(super) status: &'a ToolStatus,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ActiveToolInspector<'a> {
    pub(super) tool_call: ToolCallRecord<'a>,
    pub(super) scroll_offset: u16,
    pub(super) position: usize,
    pub(super) total: usize,
}

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
    pub(super) loop_action: String,
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
    pub(super) tool_inspector: Option<ToolInspectorState>,
    pub(super) transcript_review: TranscriptReviewState,
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
            loop_action: String::new(),
            loop_iteration: 0,
            streaming_active: false,
            spinner_frame: 0,
            dots_frame: 0,
            last_spinner_tick: Instant::now(),
            status_message: None,
            clarify_dialog: None,
            input_hint_override: None,
            staged_message: None,
            tool_inspector: None,
            transcript_review: TranscriptReviewState {
                cursor_line: 0,
                anchor_line: None,
            },
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
        if let Some(tool_call) = self.find_tool_call_mut(tool_id) {
            let MessagePart::ToolCall {
                tool_name: existing_tool_name,
                args_preview: existing_args_preview,
                ..
            } = tool_call
            else {
                return;
            };

            if existing_tool_name.is_empty() && !tool_name.is_empty() {
                *existing_tool_name = tool_name.to_string();
            }

            merge_tool_args_preview(existing_args_preview, args_preview);

            return;
        }

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

    pub(super) fn append_tool_call_args(&mut self, tool_id: &str, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        let Some(tool_call) = self.find_tool_call_mut(tool_id) else {
            return;
        };
        let MessagePart::ToolCall { args_preview, .. } = tool_call else {
            return;
        };

        merge_tool_args_preview(args_preview, chunk);
    }

    /// Finds the matching tool call by `tool_id` and transitions it to
    /// `ToolStatus::Done`.
    pub(super) fn complete_tool_call(
        &mut self,
        tool_id: &str,
        success: bool,
        output: &str,
        duration_ms: u32,
    ) {
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
                        output: output.to_string(),
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

    pub(super) fn set_transcript_cursor_to_latest(&mut self, total_lines: usize) {
        if total_lines == 0 {
            self.transcript_review.cursor_line = 0;
            self.transcript_review.anchor_line = None;
            return;
        }

        let latest_line_index = total_lines.saturating_sub(1);

        self.transcript_review.cursor_line = latest_line_index;
        self.clamp_transcript_selection_anchor(total_lines);
    }

    pub(super) fn move_transcript_cursor_up(&mut self, amount: usize, total_lines: usize) {
        if total_lines == 0 {
            self.transcript_review.cursor_line = 0;
            self.transcript_review.anchor_line = None;
            return;
        }

        let clamped_cursor = self.clamped_transcript_cursor_line(total_lines);
        let next_cursor = clamped_cursor.saturating_sub(amount);

        self.transcript_review.cursor_line = next_cursor;
        self.clamp_transcript_selection_anchor(total_lines);
    }

    pub(super) fn move_transcript_cursor_down(&mut self, amount: usize, total_lines: usize) {
        if total_lines == 0 {
            self.transcript_review.cursor_line = 0;
            self.transcript_review.anchor_line = None;
            return;
        }

        let clamped_cursor = self.clamped_transcript_cursor_line(total_lines);
        let latest_line_index = total_lines.saturating_sub(1);
        let next_cursor = clamped_cursor.saturating_add(amount).min(latest_line_index);

        self.transcript_review.cursor_line = next_cursor;
        self.clamp_transcript_selection_anchor(total_lines);
    }

    pub(super) fn jump_transcript_cursor_top(&mut self, total_lines: usize) {
        if total_lines == 0 {
            self.transcript_review.cursor_line = 0;
            self.transcript_review.anchor_line = None;
            return;
        }

        self.transcript_review.cursor_line = 0;
        self.clamp_transcript_selection_anchor(total_lines);
    }

    pub(super) fn jump_transcript_cursor_latest(&mut self, total_lines: usize) {
        self.set_transcript_cursor_to_latest(total_lines);
    }

    pub(super) fn begin_transcript_selection(&mut self) {
        if self.transcript_review.anchor_line.is_none() {
            self.transcript_review.anchor_line = Some(self.transcript_review.cursor_line);
        }
    }

    pub(super) fn toggle_transcript_selection(&mut self) -> bool {
        if self.transcript_review.anchor_line.is_some() {
            self.transcript_review.anchor_line = None;
            return false;
        }

        self.transcript_review.anchor_line = Some(self.transcript_review.cursor_line);
        true
    }

    pub(super) fn clear_transcript_selection(&mut self) -> bool {
        let had_selection = self.transcript_review.anchor_line.is_some();

        self.transcript_review.anchor_line = None;

        had_selection
    }

    pub(super) fn transcript_cursor_line(&self, total_lines: usize) -> Option<usize> {
        if total_lines == 0 {
            return None;
        }

        Some(self.clamped_transcript_cursor_line(total_lines))
    }

    pub(super) fn transcript_selection_range(&self, total_lines: usize) -> Option<(usize, usize)> {
        if total_lines == 0 {
            return None;
        }

        let anchor_line = self.transcript_review.anchor_line?;
        let cursor_line = self.clamped_transcript_cursor_line(total_lines);
        let latest_line_index = total_lines.saturating_sub(1);
        let clamped_anchor_line = anchor_line.min(latest_line_index);
        let range_start = clamped_anchor_line.min(cursor_line);
        let range_end = clamped_anchor_line.max(cursor_line);

        Some((range_start, range_end))
    }

    pub(super) fn transcript_selection_line_count(&self, total_lines: usize) -> usize {
        let selection_range = self.transcript_selection_range(total_lines);

        match selection_range {
            Some((range_start, range_end)) => range_end.saturating_sub(range_start) + 1,
            None => 0,
        }
    }

    pub(super) fn transcript_selection_line_count_hint(&self) -> usize {
        let anchor_line = match self.transcript_review.anchor_line {
            Some(anchor_line) => anchor_line,
            None => return 0,
        };
        let cursor_line = self.transcript_review.cursor_line;
        let range_start = anchor_line.min(cursor_line);
        let range_end = anchor_line.max(cursor_line);

        range_end.saturating_sub(range_start) + 1
    }

    pub(super) fn transcript_copy_text(&self, plain_lines: &[String]) -> Option<String> {
        let copy_range = self.transcript_copy_range(plain_lines.len())?;
        let (range_start, range_end) = copy_range;
        let mut selected_lines = Vec::new();

        for line in plain_lines
            .iter()
            .skip(range_start)
            .take(range_end - range_start + 1)
        {
            selected_lines.push(line.clone());
        }

        Some(selected_lines.join("\n"))
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

    pub(super) fn needs_periodic_redraw(&self) -> bool {
        self.agent_running || self.status_message.is_some()
    }

    /// Advances visible animation state and expires stale transient status
    /// messages. Returns `true` when the rendered output changed.
    pub(super) fn tick_animations(&mut self) -> bool {
        let mut changed = false;
        let elapsed = self.last_spinner_tick.elapsed().as_millis();

        if self.agent_running && elapsed >= SPINNER_INTERVAL_MS {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES;
            if elapsed >= DOTS_INTERVAL_MS {
                self.dots_frame = (self.dots_frame + 1) % DOTS_FRAMES;
            }
            self.last_spinner_tick = Instant::now();
            changed = true;
        }

        let status_expired = self
            .status_message
            .as_ref()
            .is_some_and(|(_, when)| when.elapsed().as_secs() >= 10);
        if status_expired {
            self.status_message = None;
            changed = true;
        }

        changed
    }

    pub(super) fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    pub(super) fn open_latest_tool_inspector(&mut self) -> bool {
        let tool_calls = self.collect_tool_calls();
        let latest_tool = match tool_calls.last() {
            Some(tool_call) => tool_call,
            None => return false,
        };
        let selected_tool_id = latest_tool.tool_id.to_string();

        self.tool_inspector = Some(ToolInspectorState {
            selected_tool_id,
            scroll_offset: 0,
        });

        true
    }

    pub(super) fn close_tool_inspector(&mut self) {
        self.tool_inspector = None;
    }

    pub(super) fn active_tool_inspector(&self) -> Option<ActiveToolInspector<'_>> {
        let tool_calls = self.collect_tool_calls();
        let selected_index = self.selected_tool_call_index(&tool_calls)?;
        let selected_tool = tool_calls.get(selected_index).copied()?;
        let inspector = self.tool_inspector.as_ref()?;
        let total = tool_calls.len();

        Some(ActiveToolInspector {
            tool_call: selected_tool,
            scroll_offset: inspector.scroll_offset,
            position: selected_index,
            total,
        })
    }

    pub(super) fn select_previous_tool_inspector_entry(&mut self) -> bool {
        self.move_tool_inspector_selection(ToolInspectorDirection::Previous)
    }

    pub(super) fn select_next_tool_inspector_entry(&mut self) -> bool {
        self.move_tool_inspector_selection(ToolInspectorDirection::Next)
    }

    pub(super) fn select_first_tool_inspector_entry(&mut self) -> bool {
        self.select_tool_inspector_entry_by_index(0)
    }

    pub(super) fn select_last_tool_inspector_entry(&mut self) -> bool {
        let tool_calls = self.collect_tool_calls();
        let last_index = match tool_calls.len().checked_sub(1) {
            Some(index) => index,
            None => return false,
        };

        self.select_tool_inspector_entry_by_index(last_index)
    }

    pub(super) fn scroll_tool_inspector_up(&mut self, amount: u16) {
        let inspector = match self.tool_inspector.as_mut() {
            Some(inspector) => inspector,
            None => return,
        };
        let next_offset = inspector.scroll_offset.saturating_sub(amount);

        inspector.scroll_offset = next_offset;
    }

    pub(super) fn scroll_tool_inspector_down(&mut self, amount: u16) {
        let inspector = match self.tool_inspector.as_mut() {
            Some(inspector) => inspector,
            None => return,
        };
        let next_offset = inspector.scroll_offset.saturating_add(amount);

        inspector.scroll_offset = next_offset;
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

    fn find_tool_call_mut(&mut self, tool_id: &str) -> Option<&mut MessagePart> {
        for message in self.messages.iter_mut().rev() {
            for part in message.parts.iter_mut().rev() {
                let MessagePart::ToolCall {
                    tool_id: existing_tool_id,
                    ..
                } = part
                else {
                    continue;
                };

                if existing_tool_id == tool_id {
                    return Some(part);
                }
            }
        }

        None
    }

    fn clamped_transcript_cursor_line(&self, total_lines: usize) -> usize {
        let latest_line_index = total_lines.saturating_sub(1);

        self.transcript_review.cursor_line.min(latest_line_index)
    }

    fn clamp_transcript_selection_anchor(&mut self, total_lines: usize) {
        if total_lines == 0 {
            self.transcript_review.anchor_line = None;
            return;
        }

        let latest_line_index = total_lines.saturating_sub(1);
        let clamped_anchor = self
            .transcript_review
            .anchor_line
            .map(|anchor_line| anchor_line.min(latest_line_index));

        self.transcript_review.anchor_line = clamped_anchor;
    }

    fn transcript_copy_range(&self, total_lines: usize) -> Option<(usize, usize)> {
        if total_lines == 0 {
            return None;
        }

        let selection_range = self.transcript_selection_range(total_lines);
        if selection_range.is_some() {
            return selection_range;
        }

        let cursor_line = self.clamped_transcript_cursor_line(total_lines);

        Some((cursor_line, cursor_line))
    }

    fn collect_tool_calls(&self) -> Vec<ToolCallRecord<'_>> {
        let mut tool_calls = Vec::new();

        for message in &self.messages {
            for part in &message.parts {
                let tool_call = match part {
                    MessagePart::ToolCall {
                        tool_id,
                        tool_name,
                        args_preview,
                        status,
                    } => ToolCallRecord {
                        tool_id,
                        tool_name,
                        args_preview,
                        status,
                    },
                    MessagePart::Text(_) | MessagePart::ThinkBlock(_) => continue,
                };

                tool_calls.push(tool_call);
            }
        }

        tool_calls
    }

    fn selected_tool_call_index(&self, tool_calls: &[ToolCallRecord<'_>]) -> Option<usize> {
        let inspector = self.tool_inspector.as_ref()?;
        let selected_tool_id = inspector.selected_tool_id.as_str();
        let selected_index = tool_calls
            .iter()
            .position(|tool_call| tool_call.tool_id == selected_tool_id)?;

        Some(selected_index)
    }

    fn move_tool_inspector_selection(&mut self, direction: ToolInspectorDirection) -> bool {
        let tool_calls = self.collect_tool_calls();
        let current_index = match self.selected_tool_call_index(&tool_calls) {
            Some(index) => index,
            None => return false,
        };
        let next_index = match direction {
            ToolInspectorDirection::Previous => current_index.saturating_sub(1),
            ToolInspectorDirection::Next => {
                let last_index = tool_calls.len().saturating_sub(1);
                (current_index + 1).min(last_index)
            }
        };
        if next_index == current_index {
            return false;
        }

        self.select_tool_inspector_entry_by_index(next_index)
    }

    fn select_tool_inspector_entry_by_index(&mut self, index: usize) -> bool {
        let tool_calls = self.collect_tool_calls();
        let next_tool_id = match tool_calls.get(index) {
            Some(tool_call) => tool_call.tool_id.to_string(),
            None => return false,
        };
        let inspector = match self.tool_inspector.as_mut() {
            Some(inspector) => inspector,
            None => return false,
        };

        inspector.selected_tool_id = next_tool_id;
        inspector.scroll_offset = 0;

        true
    }
}

fn merge_tool_args_preview(existing_args_preview: &mut String, incoming_args_preview: &str) {
    if incoming_args_preview.is_empty() {
        return;
    }

    if existing_args_preview.is_empty() {
        *existing_args_preview = incoming_args_preview.to_string();
        return;
    }

    if existing_args_preview == incoming_args_preview {
        return;
    }

    if incoming_args_preview.starts_with(existing_args_preview.as_str()) {
        *existing_args_preview = incoming_args_preview.to_string();
        return;
    }

    if existing_args_preview.ends_with(incoming_args_preview) {
        return;
    }

    existing_args_preview.push_str(incoming_args_preview);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolInspectorDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone)]
pub(super) struct Shell {
    pub(super) pane: Pane,
    pub(super) running: bool,
    pub(super) show_thinking: bool,
    pub(super) focus: FocusStack,
    pub(super) dirty: bool,
    pub(super) slash_command_selection: usize,
}

impl Shell {
    pub(super) fn new(session_id: &str) -> Self {
        Self {
            pane: Pane::new(session_id),
            running: true,
            show_thinking: true,
            focus: FocusStack::new(),
            dirty: true,
            slash_command_selection: 0,
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
    fn spinner_tick_advances_while_agent_running() {
        let mut pane = Pane::new("sess-1");
        pane.agent_running = true;
        let initial_frame = pane.spinner_frame;
        // Force the tick interval to have elapsed
        pane.last_spinner_tick = Instant::now() - std::time::Duration::from_millis(100);
        let changed = pane.tick_animations();
        assert!(changed);
        assert_ne!(pane.spinner_frame, initial_frame);
    }

    #[test]
    fn idle_pane_skips_periodic_redraw() {
        let pane = Pane::new("sess-1");

        assert!(!pane.needs_periodic_redraw());
    }

    #[test]
    fn status_message_keeps_periodic_redraw_until_expired() {
        let mut pane = Pane::new("sess-1");
        pane.status_message = Some((
            "recent status".to_owned(),
            Instant::now() - std::time::Duration::from_secs(11),
        ));

        assert!(pane.needs_periodic_redraw());

        let changed = pane.tick_animations();

        assert!(changed);
        assert!(pane.status_message.is_none());
        assert!(!pane.needs_periodic_redraw());
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
    fn tool_call_completion_preserves_full_output_for_inspection() {
        let mut pane = Pane::new("sess-1");
        let repeated_detail = "detail ".repeat(20);
        let full_output = format!(
            "line 1 {repeated_detail}\nline 2 with extra detail\nline 3 with trailing context"
        );

        pane.start_tool_call("t1", "read_file", "path=/foo");
        pane.complete_tool_call("t1", true, &full_output, 42);

        let last_message = pane.messages.last().expect("tool call message");
        let last_part = last_message.parts.last().expect("tool call part");

        match last_part {
            MessagePart::ToolCall { status, .. } => match status {
                ToolStatus::Done { output, .. } => {
                    assert_eq!(output.as_str(), full_output.as_str());
                }
                ToolStatus::Running { .. } => {
                    panic!("expected completed tool call output");
                }
            },
            other @ MessagePart::Text(_) | other @ MessagePart::ThinkBlock(_) => {
                panic!("expected ToolCall part, got {:?}", other)
            }
        }
    }

    #[test]
    fn repeated_tool_start_updates_existing_tool_call() {
        let mut pane = Pane::new("sess-1");

        pane.start_tool_call("t1", "file.edit", "");
        pane.start_tool_call("t1", "file.edit", "path: docs/notes.md");

        assert_eq!(pane.messages.len(), 1);

        let tool_call_count = pane.messages[0]
            .parts
            .iter()
            .filter(|part| matches!(part, MessagePart::ToolCall { .. }))
            .count();

        assert_eq!(tool_call_count, 1);

        let first_part = pane.messages[0]
            .parts
            .first()
            .expect("tool call part should exist");

        match first_part {
            MessagePart::ToolCall {
                tool_name,
                args_preview,
                status,
                ..
            } => {
                assert_eq!(tool_name, "file.edit");
                assert_eq!(args_preview, "path: docs/notes.md");
                assert!(matches!(status, ToolStatus::Running { .. }));
            }
            other @ MessagePart::Text(_) | other @ MessagePart::ThinkBlock(_) => {
                panic!("expected ToolCall part, got {:?}", other)
            }
        }
    }

    #[test]
    fn starting_same_tool_call_twice_does_not_duplicate_entry() {
        let mut pane = Pane::new("sess-1");

        pane.start_tool_call("t1", "file.write", "");
        pane.start_tool_call("t1", "file.write", "{\"path\":\"src/main.rs\"}");

        assert_eq!(pane.messages.len(), 1);

        let Some(message) = pane.messages.first() else {
            panic!("expected assistant message");
        };
        assert_eq!(message.parts.len(), 1);

        let Some(MessagePart::ToolCall {
            tool_id,
            args_preview,
            ..
        }) = message.parts.first()
        else {
            panic!("expected tool call part");
        };

        assert_eq!(tool_id, "t1");
        assert_eq!(args_preview, "{\"path\":\"src/main.rs\"}");
    }

    #[test]
    fn append_tool_call_args_extends_existing_preview() {
        let mut pane = Pane::new("sess-1");

        pane.start_tool_call("t1", "file.write", "");
        pane.append_tool_call_args("t1", "{\"path\":");
        pane.append_tool_call_args("t1", "\"src/main.rs\"}");

        let Some(message) = pane.messages.first() else {
            panic!("expected assistant message");
        };
        let Some(MessagePart::ToolCall { args_preview, .. }) = message.parts.first() else {
            panic!("expected tool call part");
        };

        assert_eq!(args_preview, "{\"path\":\"src/main.rs\"}");
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

    #[test]
    fn tool_inspector_defaults_to_none() {
        let pane = Pane::new("sess-1");

        assert!(pane.tool_inspector.is_none());
    }

    #[test]
    fn opening_latest_tool_inspector_selects_latest_tool_call() {
        let mut pane = Pane::new("sess-1");

        pane.start_tool_call("t1", "read_file", "path=/tmp/one");
        pane.complete_tool_call("t1", true, "first output", 10);
        pane.start_tool_call("t2", "shell", "ls -la");
        pane.complete_tool_call("t2", true, "second output", 20);

        let opened = pane.open_latest_tool_inspector();
        let selected_tool_id = pane
            .tool_inspector
            .as_ref()
            .map(|state| state.selected_tool_id.as_str());

        assert!(opened, "expected latest tool inspector to open");
        assert_eq!(selected_tool_id, Some("t2"));
    }

    #[test]
    fn tool_inspector_can_move_to_previous_tool_call() {
        let mut pane = Pane::new("sess-1");

        pane.start_tool_call("t1", "read_file", "path=/tmp/one");
        pane.complete_tool_call("t1", true, "first output", 10);
        pane.start_tool_call("t2", "shell", "ls -la");
        pane.complete_tool_call("t2", true, "second output", 20);
        pane.open_latest_tool_inspector();

        let moved = pane.select_previous_tool_inspector_entry();
        let selected_tool_id = pane
            .tool_inspector
            .as_ref()
            .map(|state| state.selected_tool_id.as_str());

        assert!(moved, "expected inspector to move to previous tool call");
        assert_eq!(selected_tool_id, Some("t1"));
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

    #[test]
    fn transcript_selection_range_tracks_anchor_and_cursor() {
        let mut pane = Pane::new("sess-1");

        pane.set_transcript_cursor_to_latest(8);
        pane.begin_transcript_selection();
        pane.move_transcript_cursor_up(2, 8);

        assert_eq!(pane.transcript_cursor_line(8), Some(5));
        assert_eq!(pane.transcript_selection_range(8), Some((5, 7)));
        assert_eq!(pane.transcript_selection_line_count(8), 3);
    }

    #[test]
    fn transcript_copy_text_uses_selection_range_when_present() {
        let mut pane = Pane::new("sess-1");
        let plain_lines = vec![
            "line 0".to_owned(),
            "line 1".to_owned(),
            "line 2".to_owned(),
            "line 3".to_owned(),
        ];

        pane.set_transcript_cursor_to_latest(plain_lines.len());
        pane.begin_transcript_selection();
        pane.move_transcript_cursor_up(1, plain_lines.len());

        let copied = pane
            .transcript_copy_text(plain_lines.as_slice())
            .expect("selection should copy");

        assert_eq!(copied, "line 2\nline 3");
    }

    #[test]
    fn transcript_copy_text_falls_back_to_cursor_line_without_selection() {
        let mut pane = Pane::new("sess-1");
        let plain_lines = vec!["line 0".to_owned(), "line 1".to_owned()];

        pane.set_transcript_cursor_to_latest(plain_lines.len());

        let copied = pane
            .transcript_copy_text(plain_lines.as_slice())
            .expect("cursor line should copy");

        assert_eq!(copied, "line 1");
    }
}
