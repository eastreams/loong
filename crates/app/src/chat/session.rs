use std::cmp::min;
use std::io::IsTerminal;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use console::{Key, Term};

use super::control_plane::{
    CHAT_SESSION_KIND_DELEGATE_CHILD, ChatControlPlaneApprovalSummary,
    ChatControlPlaneSessionSummary, ChatControlPlaneStore,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs, Widget, Wrap};

use super::cli_input::ConcurrentCliInputReader;
#[path = "session/render.rs"]
mod render;
use self::render::*;
#[path = "session/text.rs"]
mod text;
use self::text::*;
#[path = "session/live.rs"]
mod live;
use self::live::*;
#[path = "session/term.rs"]
mod term;
use self::term::*;
pub(crate) use self::term::{
    interactive_terminal_surface_supported, run_cli_chat_surface, run_concurrent_cli_host_surface,
};
#[path = "session/actions.rs"]
mod actions;
#[path = "session/control.rs"]
mod control;
#[path = "session/flow.rs"]
mod flow;
#[path = "session/items.rs"]
mod items;
#[path = "session/model.rs"]
mod model;
#[path = "session/view.rs"]
mod view;
use self::flow::*;
use self::items::*;
use self::model::*;
use super::*;

const ALT_SCREEN_ENTER: &str = "\x1b[?1049h";
const ALT_SCREEN_EXIT: &str = "\x1b[?1049l";
const ANSI_RESET: &str = "\x1b[0m";
const CURSOR_KEYS_NORMAL: &str = "\x1b[?1l";
const KEYPAD_NORMAL: &str = "\x1b>";
const BRACKETED_PASTE_DISABLE: &str = "\x1b[?2004l";
const CLEAR_AND_HOME: &str = "\x1b[2J\x1b[H";
const HEADER_GAP: usize = 1;
const STATUS_BAR_HEIGHT: usize = 1;
const COMPOSER_HEIGHT: usize = 4;
const SIDEBAR_WIDTH: usize = 34;
const MIN_SIDEBAR_TOTAL_WIDTH: usize = 110;
const COMMAND_OVERLAY_WIDTH: usize = 52;

pub(super) fn terminal_surface_supported() -> bool {
    Term::stdout().is_term()
}

pub(super) fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

impl ChatSessionSurface {
    fn new(runtime: CliTurnRuntime, options: CliChatOptions) -> CliResult<Self> {
        let term = Term::stdout();
        let startup_summary = ops::build_cli_chat_startup_summary(&runtime, &options)?;
        let active_provider_label = runtime
            .config
            .active_provider_id()
            .and_then(|profile_id| runtime.config.providers.get(profile_id))
            .map(|profile| {
                format!(
                    "{} / {}",
                    profile.provider.kind.display_name(),
                    profile.provider.model
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{} / {}",
                    runtime.config.provider.kind.display_name(),
                    runtime.config.provider.model
                )
            });
        let state = SurfaceState {
            startup_summary: Some(startup_summary.clone()),
            active_provider_label,
            session_title_override: None,
            last_approval: None,
            transcript: Vec::new(),
            composer: String::new(),
            composer_cursor: 0,
            history: Vec::new(),
            history_index: None,
            scroll_offset: 0,
            sticky_bottom: true,
            selected_entry: None,
            focus: SurfaceFocus::Composer,
            sidebar_visible: true,
            sidebar_tab: SidebarTab::Session,
            command_palette: None,
            overlay: Some(SurfaceOverlay::Welcome {
                screen: ops::build_cli_chat_startup_screen_spec(&startup_summary),
            }),
            live: LiveSurfaceModel::default(),
            footer_notice:
                "?: help · : command menu · M mission · Esc clear · PgUp/PgDn transcript · Tab focus".to_owned(),
            pending_turn: false,
        };
        Ok(Self {
            runtime,
            options,
            term,
            state: Arc::new(Mutex::new(state)),
        })
    }

    async fn run(self) -> CliResult<()> {
        self.run_with_shutdown(None).await
    }

    async fn run_with_shutdown(self, shutdown: Option<ConcurrentCliShutdown>) -> CliResult<()> {
        let _guard = SurfaceGuard::new(&self.term)?;
        self.render()?;

        if let Some(shutdown) = shutdown {
            self.run_concurrent_loop(shutdown).await
        } else {
            self.run_interactive_loop().await
        }
    }

    async fn run_interactive_loop(&self) -> CliResult<()> {
        loop {
            let key = self
                .term
                .read_key()
                .map_err(|error| format!("failed to read terminal key: {error}"))?;
            let action = self.handle_key(key)?;
            match action {
                SurfaceLoopAction::Continue => {}
                SurfaceLoopAction::Submit => {
                    let composer = self.lock_state().composer.clone();
                    let action = self.submit_text(composer.as_str()).await?;
                    if matches!(action, SurfaceLoopAction::Exit) {
                        break;
                    }
                }
                SurfaceLoopAction::RunCommand(command) => {
                    let action = self.submit_text(command.as_str()).await?;
                    if matches!(action, SurfaceLoopAction::Exit) {
                        break;
                    }
                }
                SurfaceLoopAction::Exit => break,
            }
        }
        Ok(())
    }

    async fn run_concurrent_loop(&self, shutdown: ConcurrentCliShutdown) -> CliResult<()> {
        let mut stdin_reader = ConcurrentCliInputReader::new()?;
        loop {
            if shutdown.is_requested() {
                break;
            }

            let next_line = tokio::select! {
                _ = shutdown.wait() => None,
                line = stdin_reader.next_line() => Some(line?),
            };

            let Some(line) = next_line else {
                break;
            };
            let Some(line) = line else {
                break;
            };

            let action = self.submit_text(line.trim()).await?;
            if matches!(action, SurfaceLoopAction::Exit) {
                break;
            }
        }

        Ok(())
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn lock_state(&self) -> std::sync::MutexGuard<'_, SurfaceState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        }
    }

    fn load_visible_worker_sessions(&self, limit: usize) -> CliResult<Vec<WorkerQueueItemSummary>> {
        let store = self.control_plane_store()?;
        let sessions = store.visible_worker_sessions(&self.runtime.session_id, limit)?;
        let mut items = Vec::new();

        for session in sessions {
            let item = WorkerQueueItemSummary::from_control_plane_summary(&session);
            items.push(item);
        }

        Ok(items)
    }

    fn load_visible_sessions(&self, limit: usize) -> CliResult<Vec<SessionQueueItemSummary>> {
        let store = self.control_plane_store()?;
        let sessions = store.visible_sessions(&self.runtime.session_id, limit)?;
        let mut items = Vec::new();

        for session in sessions {
            let item = SessionQueueItemSummary::from_control_plane_summary(&session);
            items.push(item);
        }

        Ok(items)
    }

    fn content_width(&self) -> usize {
        let (_height, width) = self.term.size();
        let width = usize::from(width);
        let sidebar_visible = self.lock_state().sidebar_visible && width >= MIN_SIDEBAR_TOTAL_WIDTH;
        width
            .saturating_sub(if sidebar_visible {
                SIDEBAR_WIDTH + 3
            } else {
                2
            })
            .max(24)
    }

    fn transcript_viewport_height_for_state(&self, state: &SurfaceState) -> usize {
        let (height, width) = self.term.size();
        let total_height = usize::from(height);
        let total_width = usize::from(width);
        let header_lines = crate::presentation::render_compact_brand_header(
            total_width.saturating_sub(2),
            &crate::presentation::BuildVersionInfo::current(),
            Some(session_subtitle(state)),
        );
        let header_height = header_lines.len();
        let reserved_height = header_height + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;

        total_height.saturating_sub(reserved_height).max(5)
    }
}

#[cfg(test)]
mod tests;
