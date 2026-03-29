pub(super) mod app_shell;
pub(super) mod composer;
pub(super) mod events;
pub(super) mod execution_band;
pub(super) mod execution_drawer;
pub(super) mod focus;
pub(super) mod layout;
pub(super) mod reducer;
pub(super) mod state;
pub(super) mod terminal;
pub(super) mod transcript;

use crate::CliResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CliTuiLaunchResult {
    Handled,
    FallbackToText { reason: String },
}

pub(super) async fn run_tui_chat(
    runtime: &super::CliTurnRuntime,
    _options: &super::CliChatOptions,
) -> CliResult<CliTuiLaunchResult> {
    match terminal::resolve_launch_mode(terminal::TerminalSupportSnapshot::capture_current()) {
        terminal::TerminalLaunch::Tui => {
            app_shell::run_placeholder_shell(runtime).await?;
            Ok(CliTuiLaunchResult::Handled)
        }
        terminal::TerminalLaunch::FallbackToText { reason } => {
            Ok(CliTuiLaunchResult::FallbackToText { reason })
        }
    }
}
