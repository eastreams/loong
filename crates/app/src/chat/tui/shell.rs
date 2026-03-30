use std::io;
use std::pin::Pin;
use std::time::Instant;

use crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::Style;
use tokio::sync::mpsc;

use crate::CliResult;
use crate::acp::AcpConversationTurnOptions;
use crate::conversation::{
    ConversationRuntimeBinding, ConversationTurnObserverHandle, ProviderErrorMode,
};

use super::commands::{self, SlashCommand};
use super::dialog::ClarifyDialog;
use super::events::UiEvent;
use super::history::PaneView;
use super::input::InputView;
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
    fn streaming_text(&self) -> &str {
        &self.streaming_text
    }
    fn is_thinking(&self) -> bool {
        self.is_thinking
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
}

impl InputView for state::Pane {
    fn agent_running(&self) -> bool {
        self.agent_running
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
    fn show_help(&self) -> bool {
        self.show_help
    }
    fn clarify_dialog(&self) -> Option<&ClarifyDialog> {
        self.pane.clarify_dialog.as_ref()
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
            action: _,
        } => {
            shell.pane.loop_state = phase;
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
        }
        UiEvent::TurnError(msg) => {
            shell.pane.agent_running = false;
            shell.pane.add_system_message(&format!("Error: {msg}"));
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

    // --- Dialog mode --------------------------------------------------
    if let Some(ref mut dialog) = shell.pane.clarify_dialog {
        #[allow(clippy::wildcard_enum_match_arm)]
        match key.code {
            KeyCode::Enter => {
                let response = dialog.response();
                shell.pane.clarify_dialog = None;
                let _ = tx.send(UiEvent::Token {
                    content: format!("\n[user chose: {response}]\n"),
                    is_thinking: false,
                });
            }
            KeyCode::Esc => {
                shell.pane.clarify_dialog = None;
            }
            KeyCode::Up => dialog.select_up(),
            KeyCode::Down => dialog.select_down(),
            KeyCode::Left => dialog.move_cursor_left(),
            KeyCode::Right => dialog.move_cursor_right(),
            KeyCode::Backspace => dialog.delete_back(),
            KeyCode::Char(ch) => dialog.insert_char(ch),
            _ => {}
        }
        return;
    }

    // --- Global shortcuts ---------------------------------------------
    #[allow(clippy::wildcard_enum_match_arm)]
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            shell.running = false;
            return;
        }
        KeyCode::Esc => {
            shell.running = false;
            return;
        }
        KeyCode::PageUp => {
            shell.pane.scroll_offset = shell.pane.scroll_offset.saturating_add(5);
            return;
        }
        KeyCode::PageDown => {
            shell.pane.scroll_offset = shell.pane.scroll_offset.saturating_sub(5);
            return;
        }
        _ => {}
    }

    // --- Enter to submit ----------------------------------------------
    if key.code == KeyCode::Enter
        && !key.modifiers.contains(KeyModifiers::SHIFT)
        && !shell.pane.agent_running
    {
        let text: String = textarea.lines().join("\n");
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        if let Some(cmd) = commands::parse(trimmed) {
            textarea.select_all();
            textarea.delete_str(usize::MAX);
            handle_slash_command(shell, cmd);
            return;
        }

        textarea.select_all();
        textarea.delete_str(usize::MAX);
        shell.pane.add_user_message(trimmed);
        shell.pane.scroll_offset = 0;
        *submit_text = Some(trimmed.to_owned());
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
            shell.show_help = !shell.show_help;
            if shell.show_help {
                let completions = commands::completions("/");
                let mut help_text = String::from("Available commands:\n");
                for (name, desc) in completions {
                    help_text.push_str(&format!("  {name:<14} {desc}\n"));
                }
                shell.pane.add_system_message(&help_text);
            }
        }
        SlashCommand::Model => {
            let model = if shell.pane.model.is_empty() {
                "(unknown)".to_owned()
            } else {
                shell.pane.model.clone()
            };
            shell.pane.add_system_message(&format!("Model: {model}"));
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

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(super) async fn run(
    runtime: &super::runtime::TuiRuntime,
    use_plain_palette: bool,
) -> CliResult<()> {
    let mut guard = TerminalGuard::enter()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<UiEvent>();

    let mut textarea = tui_textarea::TextArea::default();
    textarea.set_cursor_line_style(Style::default());

    let mut shell = state::Shell::new(&runtime.session_id);
    shell
        .pane
        .add_system_message("Welcome to LoongClaw TUI. Type a message and press Enter.");

    let palette = if use_plain_palette {
        Palette::plain()
    } else {
        Palette::dark()
    };

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut crossterm_events = EventStream::new();

    let mut turn_future: Pin<Box<dyn std::future::Future<Output = ()> + '_>> =
        Box::pin(std::future::pending());
    let mut turn_active = false;

    loop {
        // Render
        guard.draw(&shell, &textarea, &palette)?;

        // Tick spinners
        shell.pane.tick_spinner();

        let mut submit_text: Option<String> = None;

        tokio::select! {
            biased;

            // Observer/turn events from channel
            Some(event) = rx.recv() => {
                apply_ui_event(&mut shell, event);
            }

            // Crossterm terminal events
            maybe_event = crossterm_events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    apply_terminal_event(
                        &mut shell,
                        &mut textarea,
                        event,
                        &tx,
                        &mut submit_text,
                    );
                }
            }

            // Awaiting the in-flight turn to complete
            _ = &mut turn_future, if turn_active => {
                turn_active = false;
                turn_future = Box::pin(std::future::pending());
                shell.pane.agent_running = false;
            }

            // Tick timer
            _ = tick.tick() => {
                // spinner animation already handled above
            }
        }

        // Submit turn after select! releases borrows.
        if let Some(text) = submit_text.take() {
            let obs = build_tui_observer(tx.clone());
            let tx2 = tx.clone();
            turn_future = Box::pin(async move {
                let result = run_turn(runtime, &text, Some(obs)).await;
                match result {
                    Ok(reply) => {
                        // The observer streams tokens live, but if the provider
                        // returned a non-streaming response, or the final text
                        // differs from what was streamed, ensure the reply is
                        // captured as a token event so it appears in the transcript.
                        if !reply.is_empty() {
                            let _ = tx2.send(UiEvent::Token {
                                content: reply,
                                is_thinking: false,
                            });
                        }
                        let _ = tx2.send(UiEvent::ResponseDone {
                            input_tokens: 0,
                            output_tokens: 0,
                        });
                    }
                    Err(e) => {
                        let _ = tx2.send(UiEvent::TurnError(e));
                    }
                }
            });
            turn_active = true;
            shell.pane.agent_running = true;
        }

        if !shell.running {
            break;
        }
    }

    drop(guard);
    Ok(())
}
