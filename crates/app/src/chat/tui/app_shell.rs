use std::io;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::CliResult;

use super::events::UiEvent;
use super::layout::split_shell;
use super::reducer::reduce;
use super::state::{FocusTarget, UiState};

pub(crate) fn build_shell_bootstrap_state(session_id: &str) -> UiState {
    UiState::with_session_id(session_id)
}

pub(super) async fn run_placeholder_shell(runtime: &super::super::CliTurnRuntime) -> CliResult<()> {
    let mut terminal = TerminalGuard::enter()?;
    let mut events = EventStream::new();
    let mut state = build_shell_bootstrap_state(&runtime.session_id);

    terminal.draw(&state)?;

    while let Some(event) = events.next().await {
        match event {
            Ok(Event::Key(key)) => {
                let Some(ui_event) = map_key_event(key) else {
                    continue;
                };

                if reduce(&mut state, ui_event) {
                    break;
                }

                terminal.draw(&state)?;
            }
            Ok(Event::Resize(_, _)) => terminal.draw(&state)?,
            Ok(_) => {}
            Err(error) => return Err(format!("failed to read TUI input event: {error}")),
        }
    }

    Ok(())
}

fn map_key_event(event: KeyEvent) -> Option<UiEvent> {
    match event.code {
        KeyCode::Esc => Some(UiEvent::ExitRequested),
        KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(UiEvent::ExitRequested)
        }
        KeyCode::Backspace => Some(UiEvent::Backspace),
        KeyCode::Char(ch) if !event.modifiers.intersects(KeyModifiers::CONTROL) => {
            Some(UiEvent::ComposerInput(ch))
        }
        KeyCode::Enter
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
        | KeyCode::Modifier(_) => None,
    }
}

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

    fn draw(&mut self, state: &UiState) -> CliResult<()> {
        self.terminal
            .draw(|frame| render_shell(frame, state))
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

fn render_shell(frame: &mut Frame<'_>, state: &UiState) {
    let layout = split_shell(frame.area());
    let focus_label = match state.focus_target {
        FocusTarget::Composer => "composer",
    };

    let header = Paragraph::new(vec![
        Line::from("LoongClaw chat TUI"),
        Line::from(format!(
            "session={} focus={focus_label} drawer={}",
            state.session_id,
            if state.drawer_open { "open" } else { "closed" }
        )),
    ])
    .block(Block::default().title("Header").borders(Borders::ALL))
    .wrap(Wrap { trim: false });
    frame.render_widget(header, layout.header);

    let transcript = Paragraph::new(vec![
        Line::from("Transcript bootstrap"),
        Line::from("The full transcript and execution state will land in later tasks."),
        Line::from("Press Esc or Ctrl-C to exit the shell."),
    ])
    .block(Block::default().title("Conversation").borders(Borders::ALL))
    .wrap(Wrap { trim: false });
    frame.render_widget(transcript, layout.transcript);

    let execution_band = Paragraph::new(state.status_message.as_str())
        .block(Block::default().title("Execution").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(execution_band, layout.execution_band);

    let composer = Paragraph::new(format!("> {}", state.composer_text))
        .block(Block::default().title("Composer").borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(composer, layout.composer);
}
