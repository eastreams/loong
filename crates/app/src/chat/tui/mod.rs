#![allow(dead_code)]

#[cfg(test)]
pub(super) mod app_shell;
#[cfg(test)]
pub(super) mod composer;
#[cfg(test)]
pub(super) mod events;
#[cfg(test)]
pub(super) mod execution_band;
#[cfg(test)]
pub(super) mod execution_drawer;
#[cfg(test)]
pub(super) mod focus;
#[cfg(test)]
pub(super) mod layout;
#[cfg(test)]
pub(super) mod reducer;
#[cfg(test)]
pub(super) mod state;
pub(super) mod terminal;
#[cfg(test)]
pub(super) mod theme;
#[cfg(test)]
pub(super) mod transcript;

use crate::CliResult;

const PLACEHOLDER_TUI_FALLBACK_REASON: &str =
    "interactive tui runtime is not wired yet in this build";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CliTuiLaunchResult {
    FallbackToText { reason: String },
}

pub(super) async fn run_tui_chat(
    _runtime: &super::CliTurnRuntime,
    _options: &super::CliChatOptions,
) -> CliResult<CliTuiLaunchResult> {
    let launch =
        terminal::resolve_terminal_policy(terminal::TerminalSupportSnapshot::capture_current())
            .launch;
    Ok(resolve_tui_launch_result(launch))
}

fn resolve_tui_launch_result(launch: terminal::TerminalLaunch) -> CliTuiLaunchResult {
    match launch {
        terminal::TerminalLaunch::Tui => CliTuiLaunchResult::FallbackToText {
            reason: PLACEHOLDER_TUI_FALLBACK_REASON.to_owned(),
        },
        terminal::TerminalLaunch::FallbackToText { reason } => {
            CliTuiLaunchResult::FallbackToText { reason }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_terminal_still_falls_back_until_runtime_is_wired() {
        let result = resolve_tui_launch_result(terminal::TerminalLaunch::Tui);

        assert!(
            matches!(result, CliTuiLaunchResult::FallbackToText { ref reason } if reason.contains("not wired")),
            "supported TUI terminals should still fall back until the turn runtime is connected: {result:?}"
        );
    }
}
