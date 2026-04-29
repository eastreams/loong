pub mod app;
pub mod command_palette;
pub mod composer;
pub mod diff_viewer;
pub mod i18n;
pub mod markdown;
pub mod message_list;
pub mod utils;

use crate::CliResult;
use crate::chat::{CliChatOptions, ConcurrentCliHostOptions, initialize_cli_chat_surface_runtime};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::env;
use std::fmt;
use std::io::{self, IsTerminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableAlternateScroll;

impl crossterm::Command for EnableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "tried to execute EnableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableAlternateScroll;

impl crossterm::Command for DisableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "tried to execute DisableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

pub(super) fn interactive_terminal_surface_supported() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn env_value_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn mouse_capture_enabled() -> bool {
    env::var("LOONG_TUI_MOUSE_CAPTURE")
        .map(|value| env_value_truthy(value.as_str()))
        .unwrap_or(false)
}

pub(super) async fn run_cli_chat_surface(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
) -> CliResult<()> {
    let runtime =
        initialize_cli_chat_surface_runtime(config_path, session_hint, options, "cli-chat")?;

    terminal::enable_raw_mode().map_err(|e| format!("failed to enable raw mode: {}", e))?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        EnableAlternateScroll,
    )
    .map_err(|e| format!("failed to enter alternate screen: {}", e))?;
    let capture_mouse = mouse_capture_enabled();
    if capture_mouse {
        crossterm::execute!(stdout, EnableMouseCapture)
            .map_err(|e| format!("failed to enable mouse capture: {}", e))?;
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|e| format!("failed to create terminal: {}", e))?;
    terminal
        .clear()
        .map_err(|e| format!("failed to clear terminal: {}", e))?;
    terminal
        .show_cursor()
        .map_err(|e| format!("failed to show cursor: {}", e))?;

    let res = app::run_app(&mut terminal, runtime, options.clone()).await;

    terminal::disable_raw_mode().map_err(|e| format!("failed to disable raw mode: {}", e))?;
    if capture_mouse {
        crossterm::execute!(terminal.backend_mut(), DisableMouseCapture)
            .map_err(|e| format!("failed to disable mouse capture: {}", e))?;
    }
    crossterm::execute!(
        terminal.backend_mut(),
        DisableAlternateScroll,
        crossterm::terminal::LeaveAlternateScreen
    )
    .map_err(|e| format!("failed to leave alternate screen: {}", e))?;
    terminal
        .show_cursor()
        .map_err(|e| format!("failed to show cursor: {}", e))?;

    res
}

#[allow(dead_code)]
pub(super) fn run_concurrent_cli_host_surface(
    _options: &ConcurrentCliHostOptions,
) -> CliResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DisableAlternateScroll, EnableAlternateScroll, env_value_truthy};
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};

    #[test]
    fn alternate_scroll_commands_emit_expected_ansi_sequences() {
        let mut enable = String::new();
        crossterm::Command::write_ansi(&EnableAlternateScroll, &mut enable).expect("enable ansi");
        let mut disable = String::new();
        crossterm::Command::write_ansi(&DisableAlternateScroll, &mut disable)
            .expect("disable ansi");

        assert_eq!(enable, "\x1b[?1007h");
        assert_eq!(disable, "\x1b[?1007l");
    }

    #[test]
    fn mouse_capture_env_parser_keeps_native_selection_default_simple() {
        assert!(env_value_truthy("1"));
        assert!(env_value_truthy("true"));
        assert!(env_value_truthy("YES"));
        assert!(env_value_truthy("on"));
        assert!(!env_value_truthy(""));
        assert!(!env_value_truthy("0"));
        assert!(!env_value_truthy("false"));
    }

    #[test]
    fn optional_mouse_capture_commands_emit_expected_ansi_sequences() {
        let mut enable = String::new();
        crossterm::Command::write_ansi(&EnableMouseCapture, &mut enable)
            .expect("enable mouse capture ansi");
        let mut disable = String::new();
        crossterm::Command::write_ansi(&DisableMouseCapture, &mut disable)
            .expect("disable mouse capture ansi");

        assert!(enable.contains("?1000h"));
        assert!(enable.contains("?1006h"));
        assert!(disable.contains("?1000l"));
        assert!(disable.contains("?1006l"));
    }
}
