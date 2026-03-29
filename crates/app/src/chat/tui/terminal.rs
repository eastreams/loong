use std::io::IsTerminal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalSupportSnapshot {
    pub(crate) stdin_is_terminal: bool,
    pub(crate) stdout_is_terminal: bool,
    pub(crate) stderr_is_terminal: bool,
    pub(crate) term: Option<String>,
    pub(crate) color_support: bool,
}

impl TerminalSupportSnapshot {
    pub(crate) fn capture_current() -> Self {
        Self {
            stdin_is_terminal: std::io::stdin().is_terminal(),
            stdout_is_terminal: std::io::stdout().is_terminal(),
            stderr_is_terminal: std::io::stderr().is_terminal(),
            term: std::env::var("TERM").ok(),
            color_support: supports_color::on(supports_color::Stream::Stdout).is_some(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalLaunch {
    Tui,
    FallbackToText { reason: String },
}

pub(crate) fn resolve_launch_mode(snapshot: TerminalSupportSnapshot) -> TerminalLaunch {
    if !snapshot.stdin_is_terminal || !snapshot.stdout_is_terminal {
        return TerminalLaunch::FallbackToText {
            reason: "TUI requires stdin/stdout to be terminal-attached".to_owned(),
        };
    }

    if snapshot
        .term
        .as_deref()
        .is_some_and(|term| term.eq_ignore_ascii_case("dumb"))
    {
        return TerminalLaunch::FallbackToText {
            reason: "TUI requires a non-dumb terminal".to_owned(),
        };
    }

    TerminalLaunch::Tui
}
