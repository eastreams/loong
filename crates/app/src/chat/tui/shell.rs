use std::io::{self, Write};
use std::pin::Pin;
use std::time::Instant;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::{FutureExt as _, StreamExt};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::Style;
use tokio::sync::mpsc;

use crate::CliResult;
use crate::acp::AcpConversationTurnOptions;
use crate::conversation::{
    ConversationRuntimeBinding, ConversationTurnObserverHandle, ProviderErrorMode,
};

use super::boot::{TuiBootFlow, TuiBootScreen, TuiBootTransition};
use super::commands::{self, SlashCommand};
use super::dialog::ClarifyDialog;
use super::events::UiEvent;
use super::focus::{FocusLayer, FocusStack};
use super::history::{self, PaneView};
use super::input::InputView;
use super::layout;
use super::message::Message;
use super::observer::build_tui_observer;
use super::render::{self, ShellView};
use super::spinner::SpinnerView;
use super::state;
use super::status_bar::StatusBarView;
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait impls — bridge concrete state types into the render layer
// ---------------------------------------------------------------------------

impl PaneView for state::Pane {
    fn messages(&self) -> &[Message] {
        &self.messages
    }
    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }
    fn streaming_active(&self) -> bool {
        self.streaming_active
    }
    fn transcript_cursor_line(&self, total_lines: usize) -> Option<usize> {
        state::Pane::transcript_cursor_line(self, total_lines)
    }
    fn transcript_selection_range(&self, total_lines: usize) -> Option<(usize, usize)> {
        state::Pane::transcript_selection_range(self, total_lines)
    }
}

impl SpinnerView for state::Pane {
    fn agent_running(&self) -> bool {
        self.agent_running
    }
    fn spinner_frame(&self) -> usize {
        self.spinner_frame
    }
    fn dots_frame(&self) -> usize {
        self.dots_frame
    }
    fn loop_state(&self) -> &str {
        &self.loop_state
    }
    fn loop_action(&self) -> &str {
        &self.loop_action
    }
    fn loop_iteration(&self) -> u32 {
        self.loop_iteration
    }
    fn status_message(&self) -> Option<(&str, &Instant)> {
        self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
    }
}

impl StatusBarView for state::Pane {
    fn model(&self) -> &str {
        &self.model
    }
    fn input_tokens(&self) -> u32 {
        self.input_tokens
    }
    fn output_tokens(&self) -> u32 {
        self.output_tokens
    }
    fn context_length(&self) -> u32 {
        self.context_length
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }
    fn transcript_selection_line_count(&self) -> usize {
        self.transcript_selection_line_count_hint()
    }
    fn status_message(&self) -> Option<(&str, &Instant)> {
        self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
    }
}

impl InputView for state::Pane {
    fn agent_running(&self) -> bool {
        self.agent_running
    }
    fn has_staged_message(&self) -> bool {
        self.staged_message.is_some()
    }
    fn transcript_selection_line_count(&self) -> usize {
        self.transcript_selection_line_count_hint()
    }
    fn input_hint(&self) -> Option<&str> {
        self.input_hint_override.as_deref()
    }
}

impl ShellView for state::Shell {
    type Pane = state::Pane;

    fn pane(&self) -> &state::Pane {
        &self.pane
    }
    fn show_thinking(&self) -> bool {
        self.show_thinking
    }
    fn focus(&self) -> &FocusStack {
        &self.focus
    }
    fn clarify_dialog(&self) -> Option<&ClarifyDialog> {
        self.pane.clarify_dialog.as_ref()
    }
    fn tool_inspector(&self) -> Option<render::ToolInspectorView<'_>> {
        let active_tool_inspector = self.pane.active_tool_inspector()?;
        let tool_call = active_tool_inspector.tool_call;

        Some(render::ToolInspectorView {
            tool_id: tool_call.tool_id,
            tool_name: tool_call.tool_name,
            args_preview: tool_call.args_preview,
            status: tool_call.status,
            scroll_offset: active_tool_inspector.scroll_offset,
            position: active_tool_inspector.position,
            total: active_tool_inspector.total,
        })
    }
}

// ---------------------------------------------------------------------------
// RAII terminal guard
// ---------------------------------------------------------------------------

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn enter() -> CliResult<Self> {
        enable_raw_mode().map_err(|error| format!("failed to enable raw mode: {error}"))?;

        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(format!("failed to enter alternate screen: {error}"));
        }

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                return Err(format!("failed to initialize TUI terminal: {error}"));
            }
        };

        if let Err(error) = terminal.hide_cursor() {
            let _ = disable_raw_mode();
            let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
            return Err(format!("failed to hide TUI cursor: {error}"));
        }

        Ok(Self { terminal })
    }

    fn draw(
        &mut self,
        shell: &state::Shell,
        textarea: &tui_textarea::TextArea<'_>,
        palette: &Palette,
    ) -> CliResult<()> {
        self.terminal
            .draw(|frame| render::draw(frame, shell, textarea, palette))
            .map(|_| ())
            .map_err(|error| format!("failed to draw TUI frame: {error}"))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

// ---------------------------------------------------------------------------
// Streaming-tracking observer wrapper
// ---------------------------------------------------------------------------

/// Wraps a `ConversationTurnObserver` to track whether streaming tokens
/// were delivered, so the shell can send a fallback reply for non-streaming
/// providers.
struct TrackingObserver {
    inner: ConversationTurnObserverHandle,
    streamed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl crate::conversation::ConversationTurnObserver for TrackingObserver {
    fn on_phase(&self, event: crate::conversation::ConversationTurnPhaseEvent) {
        self.inner.on_phase(event);
    }

    fn on_tool(&self, event: crate::conversation::ConversationTurnToolEvent) {
        self.inner.on_tool(event);
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        if event.event_type == "text_delta" {
            self.streamed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.inner.on_streaming_token(event);
    }
}

// ---------------------------------------------------------------------------
// Turn runner
// ---------------------------------------------------------------------------

async fn run_turn(
    runtime: &super::runtime::TuiRuntime,
    input: &str,
    observer_handle: Option<ConversationTurnObserverHandle>,
) -> CliResult<String> {
    let turn_config = runtime
        .config
        .reload_provider_runtime_state_from_path(runtime.resolved_path.as_path())?;
    let acp_options = AcpConversationTurnOptions::automatic();
    runtime
        .turn_coordinator
        .handle_turn_with_address_and_acp_options_and_observer(
            &turn_config,
            &runtime.session_address,
            input,
            ProviderErrorMode::InlineMessage,
            &acp_options,
            ConversationRuntimeBinding::kernel(&runtime.kernel_ctx),
            observer_handle,
        )
        .await
}

// ---------------------------------------------------------------------------
// Event application
// ---------------------------------------------------------------------------

fn apply_ui_event(shell: &mut state::Shell, event: UiEvent) {
    match event {
        UiEvent::Tick => {
            shell.pane.tick_spinner();
        }
        UiEvent::Terminal(_) => {}
        UiEvent::Token {
            content,
            is_thinking,
        } => {
            shell.pane.append_token(&content, is_thinking);
        }
        UiEvent::ToolStart {
            tool_id,
            tool_name,
            args_preview,
        } => {
            shell
                .pane
                .start_tool_call(&tool_id, &tool_name, &args_preview);
        }
        UiEvent::ToolArgsDelta { tool_id, chunk } => {
            shell.pane.append_tool_call_args(&tool_id, &chunk);
        }
        UiEvent::ToolDone {
            tool_id,
            success,
            output,
            duration_ms,
        } => {
            shell
                .pane
                .complete_tool_call(&tool_id, success, &output, duration_ms);
        }
        UiEvent::PhaseChange {
            phase,
            iteration,
            action,
        } => {
            shell.pane.loop_state = phase;
            shell.pane.loop_action = action;
            shell.pane.loop_iteration = iteration;
        }
        UiEvent::ResponseDone {
            input_tokens,
            output_tokens,
        } => {
            shell.pane.finalize_response(input_tokens, output_tokens);
        }
        UiEvent::ClarifyRequest { question, choices } => {
            shell.pane.clarify_dialog = Some(ClarifyDialog::new(question, choices));
            shell.focus.push(FocusLayer::ClarifyDialog);
        }
        UiEvent::TurnError(msg) => {
            shell.pane.agent_running = false;
            shell.pane.add_system_message(&format!("Error: {msg}"));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryNavigationAction {
    ScrollLineUp,
    ScrollLineDown,
    ScrollHalfPageUp,
    ScrollHalfPageDown,
    ScrollPageUp,
    ScrollPageDown,
    JumpTop,
    JumpLatest,
}

fn textarea_is_empty(textarea: &tui_textarea::TextArea<'_>) -> bool {
    let lines = textarea.lines();
    let has_non_empty_line = lines.iter().any(|line| !line.is_empty());
    !has_non_empty_line
}

#[allow(clippy::wildcard_enum_match_arm)]
fn history_navigation_action(
    key: KeyEvent,
    composer_is_empty: bool,
) -> Option<HistoryNavigationAction> {
    match key.code {
        KeyCode::Up if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineUp)
        }
        KeyCode::Down if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineDown)
        }
        KeyCode::PageUp => Some(HistoryNavigationAction::ScrollPageUp),
        KeyCode::PageDown => Some(HistoryNavigationAction::ScrollPageDown),
        KeyCode::Home if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::JumpTop)
        }
        KeyCode::End if composer_is_empty && key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::JumpLatest)
        }
        _ => None,
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn transcript_navigation_action(key: KeyEvent) -> Option<HistoryNavigationAction> {
    match key.code {
        KeyCode::Up => Some(HistoryNavigationAction::ScrollLineUp),
        KeyCode::Down => Some(HistoryNavigationAction::ScrollLineDown),
        KeyCode::Char('k') if key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineUp)
        }
        KeyCode::Char('j') if key.modifiers.is_empty() => {
            Some(HistoryNavigationAction::ScrollLineDown)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(HistoryNavigationAction::ScrollHalfPageUp)
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(HistoryNavigationAction::ScrollHalfPageDown)
        }
        KeyCode::PageUp => Some(HistoryNavigationAction::ScrollPageUp),
        KeyCode::PageDown => Some(HistoryNavigationAction::ScrollPageDown),
        KeyCode::Home => Some(HistoryNavigationAction::JumpTop),
        KeyCode::End => Some(HistoryNavigationAction::JumpLatest),
        KeyCode::Char('g') if key.modifiers.is_empty() => Some(HistoryNavigationAction::JumpTop),
        KeyCode::Char('G') => Some(HistoryNavigationAction::JumpLatest),
        _ => None,
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn transcript_focus_returns_to_composer(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => true,
        KeyCode::Left => true,
        KeyCode::Right => true,
        KeyCode::Home => true,
        KeyCode::End => true,
        KeyCode::Char(_) if !key.modifiers.intersects(KeyModifiers::CONTROL) => true,
        _ => false,
    }
}

fn history_page_step(textarea: &tui_textarea::TextArea<'_>) -> u16 {
    let terminal_size = crossterm::terminal::size();
    let (width, height) = terminal_size.unwrap_or((80, 24));

    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let input_height = textarea.lines().len() as u16 + 2;
    let shell_areas = layout::compute(area, input_height);
    let history_height = shell_areas.history.height;
    let page_step = history_height.saturating_sub(1);

    page_step.max(1)
}

fn history_half_page_step(textarea: &tui_textarea::TextArea<'_>) -> u16 {
    let page_step = history_page_step(textarea);
    let half_page_step = page_step / 2;

    half_page_step.max(1)
}

fn apply_history_navigation(
    shell: &mut state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
    action: HistoryNavigationAction,
) {
    match action {
        HistoryNavigationAction::ScrollLineUp => {
            let next_offset = shell.pane.scroll_offset.saturating_add(1);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollLineDown => {
            let next_offset = shell.pane.scroll_offset.saturating_sub(1);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollHalfPageUp => {
            let half_page_step = history_half_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_add(half_page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollHalfPageDown => {
            let half_page_step = history_half_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_sub(half_page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollPageUp => {
            let page_step = history_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_add(page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::ScrollPageDown => {
            let page_step = history_page_step(textarea);
            let next_offset = shell.pane.scroll_offset.saturating_sub(page_step);
            shell.pane.scroll_offset = next_offset;
        }
        HistoryNavigationAction::JumpTop => {
            shell.pane.scroll_offset = u16::MAX;
            shell.pane.set_status("Viewing oldest output".to_owned());
        }
        HistoryNavigationAction::JumpLatest => {
            shell.pane.scroll_offset = 0;
            shell.pane.set_status("Jumped to latest output".to_owned());
        }
    }
}

fn transcript_plain_lines(shell: &state::Shell) -> Vec<String> {
    let render_width = terminal_render_width();

    history::transcript_plain_lines(shell.pane(), render_width, shell.show_thinking)
}

fn transcript_line_count(shell: &state::Shell) -> usize {
    let plain_lines = transcript_plain_lines(shell);

    plain_lines.len()
}

fn apply_transcript_navigation(
    shell: &mut state::Shell,
    textarea: &tui_textarea::TextArea<'_>,
    action: HistoryNavigationAction,
    extend_selection: bool,
) {
    let total_lines = transcript_line_count(shell);

    if extend_selection {
        shell.pane.begin_transcript_selection();
    }

    apply_history_navigation(shell, textarea, action);

    match action {
        HistoryNavigationAction::ScrollLineUp => {
            shell.pane.move_transcript_cursor_up(1, total_lines);
        }
        HistoryNavigationAction::ScrollLineDown => {
            shell.pane.move_transcript_cursor_down(1, total_lines);
        }
        HistoryNavigationAction::ScrollHalfPageUp => {
            let step = usize::from(history_half_page_step(textarea));
            shell.pane.move_transcript_cursor_up(step, total_lines);
        }
        HistoryNavigationAction::ScrollHalfPageDown => {
            let step = usize::from(history_half_page_step(textarea));
            shell.pane.move_transcript_cursor_down(step, total_lines);
        }
        HistoryNavigationAction::ScrollPageUp => {
            let step = usize::from(history_page_step(textarea));
            shell.pane.move_transcript_cursor_up(step, total_lines);
        }
        HistoryNavigationAction::ScrollPageDown => {
            let step = usize::from(history_page_step(textarea));
            shell.pane.move_transcript_cursor_down(step, total_lines);
        }
        HistoryNavigationAction::JumpTop => {
            shell.pane.jump_transcript_cursor_top(total_lines);
        }
        HistoryNavigationAction::JumpLatest => {
            shell.pane.jump_transcript_cursor_latest(total_lines);
        }
    }
}

fn open_transcript_review(shell: &mut state::Shell) {
    let total_lines = transcript_line_count(shell);

    shell.pane.set_transcript_cursor_to_latest(total_lines);
    shell.focus.focus_transcript();
    shell.pane.set_status("Transcript review mode".to_owned());
}

fn close_transcript_review(shell: &mut state::Shell) {
    shell.pane.clear_transcript_selection();
    shell.focus.focus_composer();

    shell.pane.set_status("Back to composer".to_owned());
}

fn toggle_transcript_review(shell: &mut state::Shell) {
    if shell.focus.top() == FocusLayer::Transcript {
        close_transcript_review(shell);
        return;
    }

    open_transcript_review(shell);
}

fn tool_inspector_scroll_step() -> u16 {
    let terminal_size = crossterm::terminal::size();
    let (_, height) = terminal_size.unwrap_or((80, 24));
    let available_height = height.saturating_sub(8);
    let scroll_step = available_height / 2;

    scroll_step.max(1)
}

fn open_tool_inspector(shell: &mut state::Shell) {
    let opened = shell.pane.open_latest_tool_inspector();
    if opened {
        if !shell.focus.has(FocusLayer::ToolInspector) {
            shell.focus.push(FocusLayer::ToolInspector);
        }
    } else {
        shell.pane.set_status("No tool details available".into());
    }
}

fn close_tool_inspector(shell: &mut state::Shell) {
    shell.pane.close_tool_inspector();
    if shell.focus.top() == FocusLayer::ToolInspector {
        shell.focus.pop();
    }
}

fn build_osc52_copy_sequence(text: &str) -> String {
    let encoded_text = BASE64_STANDARD.encode(text.as_bytes());

    format!("\u{1b}]52;c;{encoded_text}\u{7}")
}

fn copy_text_to_terminal_clipboard(text: &str) -> CliResult<()> {
    let copy_sequence = build_osc52_copy_sequence(text);
    let mut stdout = io::stdout();

    stdout
        .write_all(copy_sequence.as_bytes())
        .map_err(|error| format!("failed to write clipboard escape sequence: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush clipboard escape sequence: {error}"))?;

    Ok(())
}

fn copy_transcript_selection(shell: &mut state::Shell) {
    let plain_lines = transcript_plain_lines(shell);
    let copied_text = shell.pane.transcript_copy_text(plain_lines.as_slice());

    let Some(copied_text) = copied_text else {
        shell.pane.set_status("Nothing to copy".to_owned());
        return;
    };

    let copied_line_count = shell
        .pane
        .transcript_selection_line_count(plain_lines.len());
    let effective_line_count = if copied_line_count == 0 {
        1
    } else {
        copied_line_count
    };
    let line_label = if effective_line_count == 1 {
        "line"
    } else {
        "lines"
    };

    match copy_text_to_terminal_clipboard(copied_text.as_str()) {
        Ok(()) => {
            shell
                .pane
                .set_status(format!("Copied {effective_line_count} {line_label}"));
        }
        Err(error) => {
            shell.pane.set_status(format!("Copy failed: {error}"));
        }
    }
}

fn apply_terminal_event(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    event: Event,
    tx: &mpsc::UnboundedSender<UiEvent>,
    submit_text: &mut Option<String>,
) {
    let Event::Key(key) = event else {
        return;
    };

    let mut continue_in_composer = false;

    match shell.focus.top() {
        FocusLayer::ClarifyDialog => {
            if let Some(ref mut dialog) = shell.pane.clarify_dialog {
                #[allow(clippy::wildcard_enum_match_arm)]
                match key.code {
                    KeyCode::Enter => {
                        let response = dialog.response();
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                        let _ = tx.send(UiEvent::Token {
                            content: format!("\n[user chose: {response}]\n"),
                            is_thinking: false,
                        });
                    }
                    KeyCode::Esc => {
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                    }
                    KeyCode::Up => dialog.select_up(),
                    KeyCode::Down => dialog.select_down(),
                    KeyCode::Left => dialog.move_cursor_left(),
                    KeyCode::Right => dialog.move_cursor_right(),
                    KeyCode::Backspace => dialog.delete_back(),
                    KeyCode::Char(ch) => dialog.insert_char(ch),
                    _ => {}
                }
            }
            return;
        }
        FocusLayer::ToolInspector => {
            let scroll_step = tool_inspector_scroll_step();

            #[allow(clippy::wildcard_enum_match_arm)]
            #[allow(clippy::wildcard_enum_match_arm)]
            #[allow(clippy::wildcard_enum_match_arm)]
            match key.code {
                KeyCode::Esc => {
                    close_tool_inspector(shell);
                }
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = shell.pane.open_latest_tool_inspector();
                }
                KeyCode::Up => {
                    let moved = shell.pane.select_previous_tool_inspector_entry();
                    if !moved {
                        shell.pane.scroll_tool_inspector_up(1);
                    }
                }
                KeyCode::Down => {
                    let moved = shell.pane.select_next_tool_inspector_entry();
                    if !moved {
                        shell.pane.scroll_tool_inspector_down(1);
                    }
                }
                KeyCode::PageUp => {
                    shell.pane.scroll_tool_inspector_up(scroll_step);
                }
                KeyCode::PageDown => {
                    shell.pane.scroll_tool_inspector_down(scroll_step);
                }
                KeyCode::Home => {
                    let _ = shell.pane.select_first_tool_inspector_entry();
                }
                KeyCode::End => {
                    let _ = shell.pane.select_last_tool_inspector_entry();
                }
                _ => {}
            }
            return;
        }
        FocusLayer::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                shell.focus.pop();
            }
            return;
        }
        FocusLayer::Transcript => {
            let navigation_action = transcript_navigation_action(key);
            if let Some(action) = navigation_action {
                let extend_selection = key.modifiers.contains(KeyModifiers::SHIFT);
                apply_transcript_navigation(shell, textarea, action, extend_selection);
                return;
            }

            match key.code {
                KeyCode::Esc => {
                    if shell.pane.clear_transcript_selection() {
                        shell.pane.set_status("Selection cleared".to_owned());
                        return;
                    }
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Enter => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Tab if key.modifiers.is_empty() => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('q') if key.modifiers.is_empty() => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    close_transcript_review(shell);
                    return;
                }
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    open_tool_inspector(shell);
                    return;
                }
                KeyCode::Char('v') if key.modifiers.is_empty() => {
                    let selection_active = shell.pane.toggle_transcript_selection();
                    if selection_active {
                        shell.pane.set_status("Selection started".to_owned());
                    } else {
                        shell.pane.set_status("Selection cleared".to_owned());
                    }
                    return;
                }
                KeyCode::Char('y') if key.modifiers.is_empty() => {
                    copy_transcript_selection(shell);
                    return;
                }
                KeyCode::Backspace
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Delete
                | KeyCode::Insert
                | KeyCode::F(_)
                | KeyCode::Char(_)
                | KeyCode::Null
                | KeyCode::CapsLock
                | KeyCode::ScrollLock
                | KeyCode::NumLock
                | KeyCode::PrintScreen
                | KeyCode::Pause
                | KeyCode::Menu
                | KeyCode::KeypadBegin
                | KeyCode::Media(_)
                | KeyCode::Modifier(_) => {
                    if transcript_focus_returns_to_composer(key) {
                        close_transcript_review(shell);
                        continue_in_composer = true;
                    } else {
                        return;
                    }
                }
            }
        }
        FocusLayer::Composer => {
            // Fall through to global shortcuts + textarea below
        }
    }

    // --- Global shortcuts ---------------------------------------------
    let composer_is_empty = textarea_is_empty(textarea);
    let navigation_action = history_navigation_action(key, composer_is_empty);
    if let Some(action) = navigation_action {
        apply_history_navigation(shell, textarea, action);
        return;
    }

    if key.code == KeyCode::Tab && key.modifiers.is_empty() {
        toggle_transcript_review(shell);
        return;
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            shell.running = false;
            return;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            shell.show_thinking = !shell.show_thinking;
            let label = if shell.show_thinking { "on" } else { "off" };
            shell.pane.set_status(format!("Thinking display: {label}"));
            return;
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_tool_inspector(shell);
            return;
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            open_transcript_review(shell);
            return;
        }
        _ => {}
    }

    if !continue_in_composer && shell.focus.top() != FocusLayer::Composer {
        return;
    }

    // --- Escape to clear staged message --------------------------------
    if key.code == KeyCode::Esc && shell.pane.agent_running && shell.pane.staged_message.is_some() {
        shell.pane.staged_message = None;
        shell.pane.set_status("Staged message cleared".into());
        return;
    }

    // --- Enter to submit (or stage if agent is running) ---------------
    if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
        textarea.insert_newline();
        return;
    }

    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        let text: String = textarea.lines().join("\n");
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands are handled immediately regardless of agent state.
        if let Some(cmd) = commands::parse(trimmed) {
            textarea.select_all();
            textarea.delete_str(usize::MAX);
            handle_slash_command(shell, cmd);
            return;
        }

        textarea.select_all();
        textarea.delete_str(usize::MAX);

        if shell.pane.agent_running {
            // Agent is busy — stage the message (depth-1, last-wins).
            shell.pane.staged_message = Some(trimmed.to_owned());
            shell.pane.add_user_message(&format!("[queued] {trimmed}"));
            shell.pane.scroll_offset = 0;
        } else {
            // Agent is idle — submit immediately.
            shell.pane.add_user_message(trimmed);
            shell.pane.scroll_offset = 0;
            *submit_text = Some(trimmed.to_owned());
        }
        return;
    }

    // --- Everything else goes to the textarea -------------------------
    // Map crossterm key events manually to avoid version-mismatch issues
    // between the app's crossterm and tui-textarea's crossterm dependency.
    #[allow(clippy::wildcard_enum_match_arm)]
    match key.code {
        KeyCode::Char(ch) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
            textarea.insert_char(ch);
        }
        KeyCode::Backspace => {
            textarea.delete_char();
        }
        KeyCode::Left => {
            textarea.move_cursor(tui_textarea::CursorMove::Back);
        }
        KeyCode::Right => {
            textarea.move_cursor(tui_textarea::CursorMove::Forward);
        }
        KeyCode::Up => {
            textarea.move_cursor(tui_textarea::CursorMove::Up);
        }
        KeyCode::Down => {
            textarea.move_cursor(tui_textarea::CursorMove::Down);
        }
        KeyCode::Home => {
            textarea.move_cursor(tui_textarea::CursorMove::Head);
        }
        KeyCode::End => {
            textarea.move_cursor(tui_textarea::CursorMove::End);
        }
        _ => {}
    }
}

fn handle_slash_command(shell: &mut state::Shell, cmd: SlashCommand) {
    match cmd {
        SlashCommand::Exit => {
            shell.running = false;
        }
        SlashCommand::Clear => {
            shell.pane.messages.clear();
            shell.pane.add_system_message("Conversation cleared.");
        }
        SlashCommand::Help => {
            if shell.focus.has(FocusLayer::Help) {
                shell.focus.pop();
            } else {
                shell.focus.push(FocusLayer::Help);
            }
            // Help is rendered as an overlay — no transcript message needed.
        }
        SlashCommand::Model => {
            let model = if shell.pane.model.is_empty() {
                "(unknown)".to_owned()
            } else {
                shell.pane.model.clone()
            };
            shell.pane.set_status(format!("Model: {model}"));
        }
        SlashCommand::Review => {
            toggle_transcript_review(shell);
        }
        SlashCommand::ThinkOn => {
            shell.show_thinking = true;
            shell.pane.set_status("Thinking blocks enabled".into());
        }
        SlashCommand::ThinkOff => {
            shell.show_thinking = false;
            shell.pane.set_status("Thinking blocks disabled".into());
        }
        SlashCommand::Unknown(name) => {
            shell
                .pane
                .add_system_message(&format!("Unknown command: {name}"));
        }
    }
}

fn terminal_render_width() -> usize {
    match crossterm::terminal::size() {
        Ok((width, _)) => usize::from(width.max(40)),
        Err(_) => 80,
    }
}

fn replace_textarea_contents(textarea: &mut tui_textarea::TextArea<'_>, value: &str) {
    textarea.select_all();
    textarea.delete_str(usize::MAX);

    if !value.is_empty() {
        textarea.insert_str(value);
    }
}

fn take_textarea_submission(textarea: &mut tui_textarea::TextArea<'_>) -> String {
    let text = textarea.lines().join("\n");
    textarea.select_all();
    textarea.delete_str(usize::MAX);
    text
}

fn apply_boot_screen(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    screen: &TuiBootScreen,
) {
    shell.pane.show_surface_lines(&screen.lines);
    shell.pane.input_hint_override = Some(screen.prompt_hint.clone());
    shell.pane.agent_running = false;
    shell.pane.scroll_offset = u16::MAX;
    replace_textarea_contents(textarea, &screen.initial_value);
}

fn activate_chat_surface(
    shell: &mut state::Shell,
    runtime: &super::runtime::TuiRuntime,
    system_message: Option<String>,
) {
    shell.pane.messages.clear();
    shell.pane.model = runtime.model_label.clone();
    shell.pane.context_length = state::context_length_for_model(&runtime.model_label);
    shell.pane.clear_input_hint_override();

    if let Some(system_message) = system_message {
        shell.pane.add_system_message(&system_message);
    }

    shell
        .pane
        .add_system_message("Welcome to LoongClaw TUI. Type a message and press Enter.");
}

fn handle_boot_key_event(
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    key: KeyEvent,
    tx: &mpsc::UnboundedSender<UiEvent>,
    boot_escape_submit: Option<&str>,
    submit_text: &mut Option<String>,
) {
    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    if is_ctrl_c {
        shell.running = false;
        return;
    }

    let is_escape = key.code == KeyCode::Esc;
    if is_escape {
        *submit_text = boot_escape_submit.map(str::to_owned);
        return;
    }

    let is_submit = key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT);
    if is_submit {
        let text = take_textarea_submission(textarea);
        *submit_text = Some(text);
        return;
    }

    let forwarded_event = Event::Key(key);
    apply_terminal_event(shell, textarea, forwarded_event, tx, submit_text);
}

fn apply_boot_transition(
    transition: TuiBootTransition,
    boot_flow: &mut Option<Box<dyn TuiBootFlow>>,
    boot_escape_submit: &mut Option<String>,
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
) -> CliResult<()> {
    match transition {
        TuiBootTransition::Screen(screen) => {
            *boot_escape_submit = screen.escape_submit.clone();
            apply_boot_screen(shell, textarea, &screen);
        }
        TuiBootTransition::StartChat { system_message } => {
            if owned_runtime.is_none() {
                let runtime = super::runtime::initialize(config_path, session_hint)?;
                let shared_runtime = std::sync::Arc::new(runtime);
                *owned_runtime = Some(shared_runtime);
            }

            let active_runtime = resolve_active_runtime(owned_runtime.as_ref());
            let Some(runtime) = active_runtime else {
                return Err("failed to initialize TUI runtime after boot flow".to_owned());
            };

            *boot_flow = None;
            *boot_escape_submit = None;
            activate_chat_surface(shell, runtime.as_ref(), system_message);
            replace_textarea_contents(textarea, "");
        }
        TuiBootTransition::Exit => {
            shell.running = false;
        }
    }

    Ok(())
}

async fn submit_boot_flow_input(
    boot_flow: &mut Option<Box<dyn TuiBootFlow>>,
    boot_escape_submit: &mut Option<String>,
    owned_runtime: &mut Option<std::sync::Arc<super::runtime::TuiRuntime>>,
    shell: &mut state::Shell,
    textarea: &mut tui_textarea::TextArea<'_>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
    text: &str,
) -> CliResult<()> {
    let width = terminal_render_width();
    let input = text.to_owned();

    let Some(flow) = boot_flow.as_mut() else {
        return Err("internal TUI state error: boot flow missing during submit".to_owned());
    };

    let transition = flow.submit(input, width).await?;

    apply_boot_transition(
        transition,
        boot_flow,
        boot_escape_submit,
        owned_runtime,
        shell,
        textarea,
        config_path,
        session_hint,
    )?;

    Ok(())
}

fn resolve_active_runtime(
    owned_runtime: Option<&std::sync::Arc<super::runtime::TuiRuntime>>,
) -> Option<std::sync::Arc<super::runtime::TuiRuntime>> {
    owned_runtime.cloned()
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(super) async fn run(
    runtime: &super::runtime::TuiRuntime,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    run_inner(Some(runtime.clone()), None, None, None, None, palette_hint).await
}

pub(super) async fn run_lazy(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    boot_flow: Option<Box<dyn TuiBootFlow>>,
    initial_system_message: Option<String>,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    run_inner(
        None,
        config_path,
        session_hint,
        boot_flow,
        initial_system_message,
        palette_hint,
    )
    .await
}

fn prepare_chat_turn_future(
    runtime: std::sync::Arc<super::runtime::TuiRuntime>,
    text: String,
    tx: mpsc::UnboundedSender<UiEvent>,
) -> Pin<Box<dyn std::future::Future<Output = ()>>> {
    let obs = build_tui_observer(tx.clone());
    let tx2 = tx;
    let streamed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let streamed_flag = streamed.clone();
    let tracking_obs = TrackingObserver {
        inner: obs,
        streamed: streamed_flag,
    };
    let tracking_handle: crate::conversation::ConversationTurnObserverHandle =
        std::sync::Arc::new(tracking_obs);

    Box::pin(async move {
        let result = run_turn(runtime.as_ref(), text.as_str(), Some(tracking_handle)).await;
        match result {
            Ok(reply) => {
                if !streamed.load(std::sync::atomic::Ordering::Relaxed) && !reply.is_empty() {
                    let _ = tx2.send(UiEvent::Token {
                        content: reply,
                        is_thinking: false,
                    });
                    let _ = tx2.send(UiEvent::ResponseDone {
                        input_tokens: 0,
                        output_tokens: 0,
                    });
                }
            }
            Err(error) => {
                let _ = tx2.send(UiEvent::TurnError(error));
            }
        }
    })
}

async fn run_inner(
    initial_runtime: Option<super::runtime::TuiRuntime>,
    config_path: Option<&str>,
    session_hint: Option<&str>,
    mut boot_flow: Option<Box<dyn TuiBootFlow>>,
    initial_system_message: Option<String>,
    palette_hint: super::terminal::PaletteHint,
) -> CliResult<()> {
    let mut guard = TerminalGuard::enter()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();

    let mut textarea = tui_textarea::TextArea::default();
    textarea.set_cursor_line_style(Style::default());

    let session_id = initial_runtime
        .as_ref()
        .map(|runtime| runtime.session_id.as_str())
        .or_else(|| {
            session_hint
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("default");
    let mut shell = state::Shell::new(session_id);

    let mut owned_runtime = initial_runtime.map(std::sync::Arc::new);
    if boot_flow.is_none() {
        if let Some(runtime) = resolve_active_runtime(owned_runtime.as_ref()) {
            activate_chat_surface(&mut shell, runtime.as_ref(), initial_system_message.clone());
        } else {
            let runtime = super::runtime::initialize(config_path, session_hint)?;
            activate_chat_surface(&mut shell, &runtime, initial_system_message.clone());
            owned_runtime = Some(std::sync::Arc::new(runtime));
        }
    }

    let palette = match palette_hint {
        super::terminal::PaletteHint::Dark => Palette::dark(),
        super::terminal::PaletteHint::Light => Palette::light(),
        super::terminal::PaletteHint::Plain => Palette::plain(),
    };

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut crossterm_events = EventStream::new();

    let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()> + '_>> =
        Box::pin(std::future::pending());
    let mut turn_active = false;
    let mut boot_escape_submit: Option<String> = None;

    if let Some(flow) = boot_flow.as_mut() {
        let width = terminal_render_width();
        let screen = flow.begin(width)?;
        boot_escape_submit = screen.escape_submit.clone();
        apply_boot_screen(&mut shell, &mut textarea, &screen);
    }

    if shell.dirty {
        // Render a deterministic first frame before the async event loop starts
        // so PTY clients observe a stable fullscreen surface immediately.
        shell.pane.tick_spinner();
        guard.draw(&shell, &textarea, &palette)?;
        shell.dirty = false;
    }

    loop {
        // ── Phase 1: Drain all pending events (non-blocking) ──────────

        let mut submit_text: Option<String> = None;

        // Drain observer channel
        while let Ok(event) = rx.try_recv() {
            apply_ui_event(&mut shell, event);
            shell.dirty = true;
        }

        // Drain crossterm terminal events
        {
            while let Some(maybe_event) = crossterm_events.next().now_or_never().flatten() {
                if let Ok(event) = maybe_event {
                    let mut submit_text_drain: Option<String> = None;
                    if boot_flow.is_some() {
                        if let Event::Key(key) = event {
                            let boot_escape_submit = boot_escape_submit.as_deref();
                            handle_boot_key_event(
                                &mut shell,
                                &mut textarea,
                                key,
                                &tx,
                                boot_escape_submit,
                                &mut submit_text_drain,
                            );
                        }
                    } else {
                        apply_terminal_event(
                            &mut shell,
                            &mut textarea,
                            event,
                            &tx,
                            &mut submit_text_drain,
                        );
                    }
                    shell.dirty = true;

                    if submit_text_drain.is_some() {
                        submit_text = submit_text_drain;
                    }
                }
            }
        }

        // Check turn completion (non-blocking)
        if turn_active {
            let waker = futures_util::task::noop_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            if turn_future.as_mut().poll(&mut cx).is_ready() {
                turn_active = false;
                turn_future = Box::pin(std::future::pending());
                shell.pane.agent_running = false;
                shell.dirty = true;
                // Auto-submit staged message if one was queued.
                if let Some(staged) = shell.pane.staged_message.take() {
                    shell
                        .pane
                        .set_status("Sending queued message...".to_string());
                    submit_text = Some(staged);
                }
            }
        }

        // Submit turn if drain phase produced one
        if let Some(ref text) = submit_text.take() {
            if boot_flow.is_some() {
                submit_boot_flow_input(
                    &mut boot_flow,
                    &mut boot_escape_submit,
                    &mut owned_runtime,
                    &mut shell,
                    &mut textarea,
                    config_path,
                    session_hint,
                    text,
                )
                .await?;
                shell.dirty = true;
            } else if let Some(runtime) = resolve_active_runtime(owned_runtime.as_ref()) {
                turn_future = prepare_chat_turn_future(runtime, text.to_string(), tx.clone());
                turn_active = true;
                shell.pane.agent_running = true;
            }
        }

        // ── Phase 2: Render (only when dirty) ─────────────────────────
        if shell.dirty {
            shell.pane.tick_spinner();
            guard.draw(&shell, &textarea, &palette)?;
            shell.dirty = false;
        }

        if !shell.running {
            break;
        }

        // ── Phase 3: Sleep until next event or tick ───────────────────
        let mut submit_text: Option<String> = None;

        tokio::select! {
            biased;

            Some(event) = rx.recv() => {
                apply_ui_event(&mut shell, event);
                shell.dirty = true;
            }

            maybe_event = crossterm_events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    if boot_flow.is_some() {
                        if let Event::Key(key) = event {
                            let boot_escape_submit = boot_escape_submit.as_deref();
                            handle_boot_key_event(
                                &mut shell,
                                &mut textarea,
                                key,
                                &tx,
                                boot_escape_submit,
                                &mut submit_text,
                            );
                        }
                    } else {
                        apply_terminal_event(
                            &mut shell,
                            &mut textarea,
                            event,
                            &tx,
                            &mut submit_text,
                        );
                    }
                    shell.dirty = true;
                }
            }

            _ = &mut turn_future, if turn_active => {
                turn_active = false;
                turn_future = Box::pin(std::future::pending());
                shell.pane.agent_running = false;
                shell.dirty = true;
                // Auto-submit staged message if one was queued.
                if let Some(staged) = shell.pane.staged_message.take() {
                    shell.pane.set_status("Sending queued message...".to_string());
                    submit_text = Some(staged);
                }
            }

            _ = tick.tick() => {
                shell.dirty = true; // tick always triggers render
            }
        }

        // Submit turn after select! releases borrows
        if let Some(ref text) = submit_text.take() {
            if boot_flow.is_some() {
                submit_boot_flow_input(
                    &mut boot_flow,
                    &mut boot_escape_submit,
                    &mut owned_runtime,
                    &mut shell,
                    &mut textarea,
                    config_path,
                    session_hint,
                    text,
                )
                .await?;
                shell.dirty = true;
            } else if let Some(runtime) = resolve_active_runtime(owned_runtime.as_ref()) {
                turn_future = prepare_chat_turn_future(runtime, text.to_string(), tx.clone());
                turn_active = true;
                shell.pane.agent_running = true;
            }
        }
    }

    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossterm::event::{KeyEventKind, KeyEventState};

    fn plain_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn end_key_routes_to_history_when_composer_is_empty() {
        let key = plain_key(KeyCode::End);
        let action = history_navigation_action(key, true);

        assert_eq!(action, Some(HistoryNavigationAction::JumpLatest));
    }

    #[test]
    fn end_key_stays_with_input_when_composer_has_text() {
        let key = plain_key(KeyCode::End);
        let action = history_navigation_action(key, false);

        assert_eq!(action, None);
    }

    #[test]
    fn home_key_routes_to_history_when_composer_is_empty() {
        let key = plain_key(KeyCode::Home);
        let action = history_navigation_action(key, true);

        assert_eq!(action, Some(HistoryNavigationAction::JumpTop));
    }

    #[test]
    fn up_key_scrolls_history_when_composer_is_empty() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let up_event = Event::Key(plain_key(KeyCode::Up));

        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "Up should scroll transcript when composer is empty"
        );
    }

    #[test]
    fn down_key_scrolls_history_toward_latest_when_composer_is_empty() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let down_event = Event::Key(plain_key(KeyCode::Down));

        shell.pane.scroll_offset = 2;

        apply_terminal_event(&mut shell, &mut textarea, down_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "Down should move transcript toward latest output when composer is empty"
        );
    }

    #[test]
    fn shift_enter_inserts_newline_in_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let enter_event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));

        textarea.insert_str("hello");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            enter_event,
            &tx,
            &mut submit_text,
        );

        let lines = textarea.lines();

        assert_eq!(lines.len(), 2, "Shift+Enter should create a new line");
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "");
        assert!(
            submit_text.is_none(),
            "Shift+Enter should not submit composer contents"
        );
    }

    #[test]
    fn tab_switches_primary_focus_to_transcript() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
    }

    #[test]
    fn transcript_focus_scrolls_even_when_composer_has_text() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));
        let up_event = Event::Key(plain_key(KeyCode::Up));

        textarea.insert_str("draft reply");

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);
        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        let draft_text = textarea.lines().join("\n");

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(shell.pane.scroll_offset, 1);
        assert_eq!(draft_text, "draft reply");
    }

    #[test]
    fn typing_while_transcript_focused_returns_to_composer_and_keeps_draft() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let tab_event = Event::Key(plain_key(KeyCode::Tab));
        let char_event = Event::Key(plain_key(KeyCode::Char('!')));

        textarea.insert_str("draft");

        apply_terminal_event(&mut shell, &mut textarea, tab_event, &tx, &mut submit_text);
        apply_terminal_event(&mut shell, &mut textarea, char_event, &tx, &mut submit_text);

        let draft_text = textarea.lines().join("\n");

        assert_eq!(shell.focus.top(), FocusLayer::Composer);
        assert_eq!(draft_text, "draft!");
    }

    #[test]
    fn ctrl_o_without_tool_calls_sets_status_message() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);

        let status_message = shell
            .pane
            .status_message
            .as_ref()
            .map(|(msg, _)| msg.as_str());

        assert_eq!(status_message, Some("No tool details available"));
    }

    #[test]
    fn ctrl_o_with_tool_calls_opens_tool_inspector() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

        shell.pane.start_tool_call("tool-1", "shell", "ls -la");
        shell.pane.complete_tool_call("tool-1", true, "file-a", 12);

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);

        let selected_tool_id = shell
            .pane
            .tool_inspector
            .as_ref()
            .map(|state| state.selected_tool_id.as_str());

        assert_eq!(shell.focus.top(), FocusLayer::ToolInspector);
        assert_eq!(selected_tool_id, Some("tool-1"));
    }

    #[test]
    fn ctrl_r_enables_history_navigation_with_non_empty_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let up_event = Event::Key(plain_key(KeyCode::Up));

        textarea.insert_str("draft message");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(&mut shell, &mut textarea, up_event, &tx, &mut submit_text);

        assert_eq!(
            shell.pane.scroll_offset, 1,
            "review mode should allow transcript scrolling even when the composer has text"
        );
        assert_eq!(
            textarea.lines().join("\n"),
            "draft message",
            "review mode should not mutate composer contents while navigating transcript"
        );
    }

    #[test]
    fn esc_closes_tool_inspector_focus() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let open_event = Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let close_event = Event::Key(plain_key(KeyCode::Esc));

        shell.pane.start_tool_call("tool-1", "shell", "ls -la");
        shell.pane.complete_tool_call("tool-1", true, "file-a", 12);

        apply_terminal_event(&mut shell, &mut textarea, open_event, &tx, &mut submit_text);
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            close_event,
            &tx,
            &mut submit_text,
        );

        assert_eq!(shell.focus.top(), FocusLayer::Composer);
        assert!(shell.pane.tool_inspector.is_none());
    }

    #[test]
    fn shift_up_in_transcript_focus_starts_selection() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let shift_up_event = Event::Key(modified_key(KeyCode::Up, KeyModifiers::SHIFT));

        shell.pane.add_system_message("line 1");
        shell.pane.add_system_message("line 2");
        shell.pane.add_system_message("line 3");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            shift_up_event,
            &tx,
            &mut submit_text,
        );

        let total_lines = transcript_line_count(&shell);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(
            shell.pane.transcript_selection_range(total_lines),
            Some((4, 5))
        );
    }

    #[test]
    fn esc_clears_transcript_selection_before_returning_to_composer() {
        let mut shell = state::Shell::new("test");
        let mut textarea = tui_textarea::TextArea::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut submit_text: Option<String> = None;
        let review_event = Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let selection_event = Event::Key(plain_key(KeyCode::Char('v')));
        let esc_event = Event::Key(plain_key(KeyCode::Esc));

        shell.pane.add_system_message("line 1");

        apply_terminal_event(
            &mut shell,
            &mut textarea,
            review_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(
            &mut shell,
            &mut textarea,
            selection_event,
            &tx,
            &mut submit_text,
        );
        apply_terminal_event(&mut shell, &mut textarea, esc_event, &tx, &mut submit_text);

        let total_lines = transcript_line_count(&shell);

        assert_eq!(shell.focus.top(), FocusLayer::Transcript);
        assert_eq!(shell.pane.transcript_selection_range(total_lines), None);
    }

    #[test]
    fn osc52_copy_sequence_encodes_clipboard_payload() {
        let sequence = build_osc52_copy_sequence("hello");

        assert!(sequence.starts_with("\u{1b}]52;c;"));
        assert!(sequence.ends_with('\u{7}'));
        assert!(sequence.contains("aGVsbG8="));
    }
}
