use std::io::IsTerminal;

// ---------------------------------------------------------------------------
// Snapshot-based terminal detection
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Launch decision
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalLaunch {
    Tui,
    FallbackToText { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalPolicy {
    pub(crate) launch: TerminalLaunch,
    pub(crate) use_plain_palette: bool,
}

/// Pure-function launch-mode resolver.  Operates on an explicit snapshot so
/// callers (including tests) can evaluate the decision deterministically
/// without probing the live environment.
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

/// Combines launch-mode resolution with palette selection.
pub(crate) fn resolve_terminal_policy(snapshot: TerminalSupportSnapshot) -> TerminalPolicy {
    let use_plain_palette = !snapshot.color_support;
    let launch = resolve_launch_mode(snapshot);

    TerminalPolicy {
        launch,
        use_plain_palette,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::tui::theme::SemanticPalette;
    use ratatui::style::Color;

    #[test]
    fn default_semantic_palette_shape_is_conservative() {
        let palette = SemanticPalette::default();

        assert_eq!(palette.text, Color::White);
        assert_eq!(palette.border, Color::DarkGray);
        assert_eq!(palette.accent, Color::Cyan);
        assert_eq!(palette.warning, Color::Yellow);
        assert_eq!(palette.error, Color::Red);
    }

    #[test]
    fn terminal_policy_chooses_text_mode_when_tty_preconditions_fail() {
        let policy = resolve_terminal_policy(TerminalSupportSnapshot {
            stdin_is_terminal: false,
            stdout_is_terminal: false,
            stderr_is_terminal: false,
            term: Some("xterm-256color".to_owned()),
            color_support: false,
        });

        assert!(matches!(
            policy.launch,
            TerminalLaunch::FallbackToText { .. }
        ));
        assert!(policy.use_plain_palette);
    }

    #[test]
    fn terminal_policy_chooses_tui_when_all_conditions_met() {
        let policy = resolve_terminal_policy(TerminalSupportSnapshot {
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            stderr_is_terminal: true,
            term: Some("xterm-256color".to_owned()),
            color_support: true,
        });

        assert!(matches!(policy.launch, TerminalLaunch::Tui));
        assert!(!policy.use_plain_palette);
    }

    #[test]
    fn dumb_terminal_falls_back() {
        let launch = resolve_launch_mode(TerminalSupportSnapshot {
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            stderr_is_terminal: true,
            term: Some("dumb".to_owned()),
            color_support: false,
        });

        assert!(matches!(launch, TerminalLaunch::FallbackToText { .. }));
    }

    #[test]
    fn missing_term_env_does_not_block_launch() {
        let launch = resolve_launch_mode(TerminalSupportSnapshot {
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            stderr_is_terminal: true,
            term: None,
            color_support: true,
        });

        assert!(matches!(launch, TerminalLaunch::Tui));
    }

    #[test]
    fn no_color_support_still_launches_with_plain_palette() {
        let policy = resolve_terminal_policy(TerminalSupportSnapshot {
            stdin_is_terminal: true,
            stdout_is_terminal: true,
            stderr_is_terminal: true,
            term: Some("xterm".to_owned()),
            color_support: false,
        });

        assert!(matches!(policy.launch, TerminalLaunch::Tui));
        assert!(policy.use_plain_palette);
    }
}
