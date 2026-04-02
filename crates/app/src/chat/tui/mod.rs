#![allow(dead_code)]

pub(super) mod app_shell;
pub mod boot;
pub(super) mod commands;
pub(super) mod dialog;
pub(super) mod events;
pub(super) mod focus;
pub(super) mod history;
pub(super) mod input;
pub(super) mod layout;
pub(super) mod message;
pub(super) mod observer;
pub(super) mod render;
pub(crate) mod runtime;
pub(super) mod shell;
pub(super) mod spinner;
pub(super) mod state;
pub(super) mod status_bar;
pub(super) mod terminal;
pub(super) mod theme;

use crate::CliResult;
pub use boot::{TuiBootFlow, TuiBootScreen, TuiBootTransition};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CliTuiLaunchResult {
    Entered,
    FallbackToText { reason: String },
}

/// Legacy bridge called from `run_cli_chat` when `--ui tui` is requested
/// through the existing CLI path.  Delegates to the self-contained TUI
/// runtime so that `chat.rs` internals are not leaked into the TUI module.
pub(super) async fn run_tui_chat(
    runtime: &super::CliTurnRuntime,
    _options: &super::CliChatOptions,
) -> CliResult<CliTuiLaunchResult> {
    let snapshot = terminal::TerminalSupportSnapshot::capture_current();
    let policy = terminal::resolve_terminal_policy(snapshot);

    match policy.launch {
        terminal::TerminalLaunch::Tui => {
            // Bootstrap a self-contained TuiRuntime from the already-loaded
            // CliTurnRuntime fields so shell::run only depends on our own type.
            let tui_rt = runtime::TuiRuntime {
                resolved_path: runtime.resolved_path.clone(),
                config: runtime.config.clone(),
                session_id: runtime.session_id.clone(),
                session_address: runtime.session_address.clone(),
                turn_coordinator: crate::conversation::ConversationTurnCoordinator::new(),
                kernel_ctx: runtime.kernel_ctx.clone(),
                model_label: runtime
                    .config
                    .provider
                    .resolved_model()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "auto".to_owned()),
            };
            shell::run(&tui_rt, policy.palette_hint).await?;
            Ok(CliTuiLaunchResult::Entered)
        }
        terminal::TerminalLaunch::FallbackToText { reason } => {
            Ok(CliTuiLaunchResult::FallbackToText { reason })
        }
    }
}

/// Public entry point for the standalone `loong tui` command.
///
/// Initializes its own runtime from config without importing any private
/// types from `chat.rs`.
pub async fn run_tui(config_path: Option<&str>, session_hint: Option<&str>) -> CliResult<()> {
    let snapshot = terminal::TerminalSupportSnapshot::capture_current();
    let policy = terminal::resolve_terminal_policy(snapshot);

    match policy.launch {
        terminal::TerminalLaunch::Tui => {}
        terminal::TerminalLaunch::FallbackToText { reason } => {
            return Err(reason);
        }
    }

    shell::run_lazy(config_path, session_hint, None, policy.palette_hint).await
}

pub async fn run_tui_with_boot_flow(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    boot_flow: Box<dyn TuiBootFlow>,
) -> CliResult<()> {
    let snapshot = terminal::TerminalSupportSnapshot::capture_current();
    let policy = terminal::resolve_terminal_policy(snapshot);

    match policy.launch {
        terminal::TerminalLaunch::Tui => {}
        terminal::TerminalLaunch::FallbackToText { reason } => {
            return Err(reason);
        }
    }

    shell::run_lazy(
        config_path,
        session_hint,
        Some(boot_flow),
        policy.palette_hint,
    )
    .await
}
