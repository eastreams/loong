use std::cmp::min;
use std::io::IsTerminal;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use console::{Key, Term};

use super::cli_input::ConcurrentCliInputReader;
use super::*;

const ALT_SCREEN_ENTER: &str = "\x1b[?1049h";
const ALT_SCREEN_EXIT: &str = "\x1b[?1049l";
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

fn terminal_surface_allowed(stdout_is_tty: bool, stdin_is_tty: bool) -> bool {
    if !stdout_is_tty {
        return false;
    }

    if !stdin_is_tty {
        return false;
    }

    true
}

pub(super) fn interactive_terminal_surface_supported() -> bool {
    let stdout_is_tty = terminal_surface_supported();
    let stdin_is_tty = stdin_is_tty();

    terminal_surface_allowed(stdout_is_tty, stdin_is_tty)
}

pub(super) async fn run_cli_chat_surface(
    config_path: Option<&str>,
    session_hint: Option<&str>,
    options: &CliChatOptions,
) -> CliResult<()> {
    let runtime = initialize_cli_turn_runtime(config_path, session_hint, options, "cli-chat")?;
    let surface = ChatSessionSurface::new(runtime, options.clone())?;
    surface.run().await
}

pub(super) fn run_concurrent_cli_host_surface(options: &ConcurrentCliHostOptions) -> CliResult<()> {
    reject_disabled_cli_channel(&options.config)?;
    let chat_options = CliChatOptions::default();
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        options.resolved_path.clone(),
        options.config.clone(),
        Some(options.session_id.as_str()),
        &chat_options,
        "cli-chat-concurrent",
        CliSessionRequirement::RequireExplicit,
        options.initialize_runtime_environment,
    )?;
    let host_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize concurrent CLI host runtime: {error}"))?;
    let surface = ChatSessionSurface::new(runtime, chat_options)?;
    host_runtime.block_on(async {
        surface
            .run_with_shutdown(Some(options.shutdown.clone()))
            .await
    })
}

struct ChatSessionSurface {
    runtime: CliTurnRuntime,
    options: CliChatOptions,
    term: Term,
    state: Arc<Mutex<SurfaceState>>,
}

#[derive(Clone)]
struct SurfaceEntry {
    lines: Vec<String>,
}

#[derive(Clone, Default)]
struct SurfaceState {
    startup_summary: Option<operator_surfaces::CliChatStartupSummary>,
    session_title_override: Option<String>,
    transcript: Vec<SurfaceEntry>,
    composer: String,
    composer_cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    scroll_offset: usize,
    sticky_bottom: bool,
    selected_entry: Option<usize>,
    focus: SurfaceFocus,
    sidebar_visible: bool,
    sidebar_tab: SidebarTab,
    command_palette: Option<CommandPaletteState>,
    overlay: Option<SurfaceOverlay>,
    live: LiveSurfaceModel,
    footer_notice: String,
    pending_turn: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum SidebarTab {
    #[default]
    Session,
    Runtime,
    Tools,
    Help,
}

impl SidebarTab {
    fn title(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Runtime => "runtime",
            Self::Tools => "tools",
            Self::Help => "help",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Session => Self::Runtime,
            Self::Runtime => Self::Tools,
            Self::Tools => Self::Help,
            Self::Help => Self::Session,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Session => Self::Help,
            Self::Runtime => Self::Session,
            Self::Tools => Self::Runtime,
            Self::Help => Self::Tools,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CommandPaletteState {
    selected: usize,
    query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum SurfaceFocus {
    Transcript,
    #[default]
    Composer,
    Sidebar,
    CommandPalette,
}

#[derive(Clone, Debug)]
enum SurfaceOverlay {
    EntryDetails {
        entry_index: usize,
    },
    Timeline,
    ConfirmExit,
    InputPrompt {
        kind: OverlayInputKind,
        value: String,
        cursor: usize,
    },
    ApprovalPrompt {
        screen: TuiScreenSpec,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayInputKind {
    RenameSession,
    ExportTranscript,
}

impl SurfaceFocus {
    fn next(self, sidebar_visible: bool, palette_open: bool) -> Self {
        if palette_open {
            return Self::CommandPalette;
        }

        match self {
            Self::Transcript => {
                if sidebar_visible {
                    Self::Sidebar
                } else {
                    Self::Composer
                }
            }
            Self::Composer => Self::Transcript,
            Self::Sidebar => Self::Composer,
            Self::CommandPalette => Self::Composer,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Composer => "composer",
            Self::Sidebar => "sidebar",
            Self::CommandPalette => "palette",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandPaletteAction {
    Help,
    Status,
    History,
    Compact,
    Timeline,
    RenameSession,
    ExportTranscript,
    JumpLatest,
    ToggleSticky,
    ToggleSidebar,
    CycleSidebarTab,
    ClearComposer,
    Exit,
}

impl CommandPaletteAction {
    fn items() -> &'static [(&'static str, &'static str, Self)] {
        &[
            ("Help", "Open slash command reference", Self::Help),
            ("Status", "Show runtime/session status card", Self::Status),
            ("History", "Show transcript window summary", Self::History),
            ("Compact", "Run manual compaction summary", Self::Compact),
            (
                "Timeline",
                "Open the transcript navigator overlay",
                Self::Timeline,
            ),
            (
                "Rename session",
                "Set a local surface title for this session",
                Self::RenameSession,
            ),
            (
                "Export transcript",
                "Write the current transcript to a text file",
                Self::ExportTranscript,
            ),
            (
                "Jump to latest",
                "Select the newest transcript entry and stick to bottom",
                Self::JumpLatest,
            ),
            (
                "Toggle sticky scroll",
                "Pin transcript to bottom or keep manual scroll position",
                Self::ToggleSticky,
            ),
            (
                "Toggle sidebar",
                "Show or hide the right rail",
                Self::ToggleSidebar,
            ),
            (
                "Cycle rail tab",
                "Move the right rail to the next tab",
                Self::CycleSidebarTab,
            ),
            (
                "Clear composer",
                "Clear the current draft",
                Self::ClearComposer,
            ),
            ("Exit", "Leave the session surface", Self::Exit),
        ]
    }
}

fn filtered_command_palette_items(
    query: &str,
) -> Vec<(&'static str, &'static str, CommandPaletteAction)> {
    let trimmed = query.trim().to_ascii_lowercase();
    let mut items = CommandPaletteAction::items().to_vec();
    if trimmed.is_empty() {
        return items;
    }
    items.retain(|(label, detail, _)| {
        label.to_ascii_lowercase().contains(trimmed.as_str())
            || detail.to_ascii_lowercase().contains(trimmed.as_str())
    });
    items
}

#[derive(Clone, Default)]
struct LiveSurfaceModel {
    snapshot: Option<CliChatLiveSurfaceSnapshot>,
    state: CliChatLiveSurfaceState,
    last_assistant_preview: Option<String>,
    last_phase_label: String,
}

fn sync_live_surface_snapshot(live: &mut LiveSurfaceModel) {
    let snapshot = build_cli_chat_live_surface_snapshot(&live.state);
    live.snapshot = snapshot;
}

fn fallback_live_surface_snapshot() -> CliChatLiveSurfaceSnapshot {
    CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::Preparing,
        provider_round: None,
        lane: None,
        tool_call_count: 0,
        message_count: None,
        estimated_tokens: None,
        draft_preview: None,
        tools: Vec::new(),
    }
}

struct SurfaceGuard {
    term: Term,
}

impl SurfaceGuard {
    fn new(term: &Term) -> CliResult<Self> {
        term.write_str(ALT_SCREEN_ENTER)
            .map_err(|error| format!("failed to enter alternate screen: {error}"))?;
        term.hide_cursor()
            .map_err(|error| format!("failed to hide cursor: {error}"))?;
        term.clear_screen()
            .map_err(|error| format!("failed to clear screen: {error}"))?;
        Ok(Self { term: term.clone() })
    }
}

impl Drop for SurfaceGuard {
    fn drop(&mut self) {
        let _ = self.term.show_cursor();
        let _ = self.term.write_str(ALT_SCREEN_EXIT);
        let _ = self.term.flush();
    }
}

impl ChatSessionSurface {
    fn new(runtime: CliTurnRuntime, options: CliChatOptions) -> CliResult<Self> {
        let term = Term::stdout();
        let startup_summary =
            operator_surfaces::build_cli_chat_startup_summary(&runtime, &options)?;
        let mut state = SurfaceState {
            startup_summary: Some(startup_summary.clone()),
            session_title_override: None,
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
            overlay: None,
            live: LiveSurfaceModel::default(),
            footer_notice:
                "Esc clear · ↑↓ history/scroll · PgUp/PgDn transcript · Tab focus · : commands"
                    .to_owned(),
            pending_turn: false,
        };
        let render_width = detect_cli_chat_render_width();
        state.transcript.push(SurfaceEntry {
            lines: operator_surfaces::render_cli_chat_startup_lines_with_width(
                &startup_summary,
                render_width,
            ),
        });
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
    fn handle_key(&self, key: Key) -> CliResult<SurfaceLoopAction> {
        match key {
            Key::CtrlC => Ok(SurfaceLoopAction::Exit),
            Key::Escape => {
                let mut state = self.lock_state();
                if matches!(state.overlay, Some(SurfaceOverlay::ConfirmExit)) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.overlay.is_some() {
                    state.overlay = None;
                    if state.command_palette.is_none() {
                        state.focus = SurfaceFocus::Transcript;
                    }
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.command_palette.is_some() {
                    state.command_palette = None;
                    state.focus = SurfaceFocus::Composer;
                    state.composer.clear();
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.composer.is_empty() {
                    state.overlay = Some(SurfaceOverlay::ConfirmExit);
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                state.composer.clear();
                state.composer_cursor = 0;
                state.history_index = None;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Tab => {
                let mut state = self.lock_state();
                if state.command_palette.is_some() {
                    state.command_palette = None;
                    state.focus = SurfaceFocus::Composer;
                } else {
                    state.focus = state
                        .focus
                        .next(state.sidebar_visible, state.command_palette.is_some());
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::BackTab => {
                let mut state = self.lock_state();
                state.sidebar_tab = state.sidebar_tab.previous();
                state.focus = SurfaceFocus::Sidebar;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowUp => {
                let mut state = self.lock_state();
                if let Some(palette) = state.command_palette.as_mut() {
                    palette.selected = palette.selected.saturating_sub(1);
                } else if state.focus == SurfaceFocus::Composer
                    && state.composer.contains('\n')
                    && !state.composer.is_empty()
                {
                    state.composer_cursor =
                        move_cursor_vertically(&state.composer, state.composer_cursor, -1);
                } else if state.focus == SurfaceFocus::Transcript || state.composer.is_empty() {
                    state.scroll_offset = state.scroll_offset.saturating_add(3);
                    state.sticky_bottom = false;
                    let selected = state
                        .selected_entry
                        .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
                    state.selected_entry = Some(selected.saturating_sub(1));
                } else if !state.history.is_empty() {
                    let next_index = match state.history_index {
                        Some(index) => index.saturating_sub(1),
                        None => state.history.len().saturating_sub(1),
                    };
                    state.history_index = Some(next_index);
                    if let Some(entry) = state.history.get(next_index) {
                        state.composer = entry.clone();
                        state.composer_cursor = state.composer.chars().count();
                    }
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowDown => {
                let mut state = self.lock_state();
                if let Some(palette) = state.command_palette.as_mut() {
                    let max_index = filtered_command_palette_items(&palette.query)
                        .len()
                        .saturating_sub(1);
                    palette.selected = min(palette.selected.saturating_add(1), max_index);
                } else if state.focus == SurfaceFocus::Composer
                    && state.composer.contains('\n')
                    && !state.composer.is_empty()
                {
                    state.composer_cursor =
                        move_cursor_vertically(&state.composer, state.composer_cursor, 1);
                } else if state.focus == SurfaceFocus::Transcript || state.composer.is_empty() {
                    state.scroll_offset = state.scroll_offset.saturating_sub(3);
                    if state.scroll_offset == 0 {
                        state.sticky_bottom = true;
                    }
                    let next_selected = state
                        .selected_entry
                        .unwrap_or_else(|| state.transcript.len().saturating_sub(1))
                        .saturating_add(1);
                    state.selected_entry =
                        Some(min(next_selected, state.transcript.len().saturating_sub(1)));
                } else if let Some(index) = state.history_index {
                    let next_index = index.saturating_add(1);
                    if next_index >= state.history.len() {
                        state.history_index = None;
                        state.composer.clear();
                    } else {
                        state.history_index = Some(next_index);
                        if let Some(entry) = state.history.get(next_index) {
                            state.composer = entry.clone();
                            state.composer_cursor = state.composer.chars().count();
                        }
                    }
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Home => {
                let mut state = self.lock_state();
                if state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = 0;
                } else {
                    state.sidebar_tab = state.sidebar_tab.previous();
                    state.focus = SurfaceFocus::Sidebar;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::End => {
                let mut state = self.lock_state();
                if state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = state.composer.chars().count();
                } else {
                    state.sidebar_tab = state.sidebar_tab.next();
                    state.focus = SurfaceFocus::Sidebar;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::PageUp => {
                let mut state = self.lock_state();
                state.scroll_offset = state.scroll_offset.saturating_add(10);
                state.sticky_bottom = false;
                state.focus = SurfaceFocus::Transcript;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::PageDown => {
                let mut state = self.lock_state();
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
                if state.scroll_offset == 0 {
                    state.sticky_bottom = true;
                }
                state.focus = SurfaceFocus::Transcript;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Backspace => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    remove_char_before_cursor(value, cursor);
                } else if let Some(palette) = state.command_palette.as_mut() {
                    palette.query.pop();
                    let max_index = filtered_command_palette_items(&palette.query)
                        .len()
                        .saturating_sub(1);
                    palette.selected = min(palette.selected, max_index);
                } else {
                    let mut cursor = state.composer_cursor;
                    remove_char_before_cursor(&mut state.composer, &mut cursor);
                    state.composer_cursor = cursor;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowLeft => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    *cursor = cursor.saturating_sub(1).min(value.chars().count());
                } else if state.command_palette.is_none() && state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = state.composer_cursor.saturating_sub(1);
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowRight => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    *cursor = min(cursor.saturating_add(1), value.chars().count());
                } else if state.command_palette.is_none() && state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = min(
                        state.composer_cursor.saturating_add(1),
                        state.composer.chars().count(),
                    );
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Enter => {
                {
                    let state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::ConfirmExit)) {
                        return Ok(SurfaceLoopAction::Exit);
                    }
                }
                {
                    let overlay_input = {
                        let state = self.lock_state();
                        match state.overlay.as_ref() {
                            Some(SurfaceOverlay::InputPrompt {
                                kind,
                                value,
                                cursor: _,
                            }) => Some((*kind, value.clone())),
                            _ => None,
                        }
                    };
                    if let Some((kind, value)) = overlay_input {
                        self.submit_input_overlay(kind, value)?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::ApprovalPrompt { .. })) {
                        let response = state.composer.trim().to_owned();
                        if !response.is_empty() {
                            state.overlay = None;
                            state.focus = SurfaceFocus::Composer;
                            return Ok(SurfaceLoopAction::Submit);
                        }
                    }
                }
                {
                    let mut state = self.lock_state();
                    if state.command_palette.is_none()
                        && state.focus == SurfaceFocus::Composer
                        && should_continue_multiline_at_cursor(
                            &state.composer,
                            state.composer_cursor,
                        )
                    {
                        let mut cursor = state.composer_cursor;
                        remove_char_before_cursor(&mut state.composer, &mut cursor);
                        insert_char_at_cursor(&mut state.composer, &mut cursor, '\n');
                        state.composer_cursor = cursor;
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                let maybe_action = self
                    .lock_state()
                    .command_palette
                    .as_ref()
                    .and_then(|palette| {
                        filtered_command_palette_items(&palette.query)
                            .get(palette.selected)
                            .map(|item| item.2)
                    });
                if let Some(action) = maybe_action {
                    return self.execute_palette_action(action);
                }
                {
                    let mut state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::Timeline)) {
                        let entry_index = state
                            .selected_entry
                            .or_else(|| state.transcript.len().checked_sub(1));
                        if let Some(entry_index) = entry_index {
                            state.overlay = Some(SurfaceOverlay::EntryDetails { entry_index });
                            drop(state);
                            self.render()?;
                            return Ok(SurfaceLoopAction::Continue);
                        }
                    }
                }
                let maybe_overlay = {
                    let state = self.lock_state();
                    if state.focus == SurfaceFocus::Transcript {
                        state
                            .selected_entry
                            .or_else(|| state.transcript.len().checked_sub(1))
                    } else {
                        None
                    }
                };
                if let Some(entry_index) = maybe_overlay {
                    let mut state = self.lock_state();
                    state.overlay = Some(SurfaceOverlay::EntryDetails { entry_index });
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                Ok(SurfaceLoopAction::Submit)
            }
            Key::Char(character) => {
                let mut state = self.lock_state();
                if character == ':' && state.composer.is_empty() {
                    state.command_palette = Some(CommandPaletteState::default());
                    state.focus = SurfaceFocus::CommandPalette;
                } else if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    if !character.is_control() {
                        insert_char_at_cursor(value, cursor, character);
                    }
                } else if let Some(palette) = state.command_palette.as_mut() {
                    if !character.is_control() {
                        palette.query.push(character);
                        let max_index = filtered_command_palette_items(&palette.query)
                            .len()
                            .saturating_sub(1);
                        palette.selected = min(palette.selected, max_index);
                    }
                } else if character == ']' && state.composer.is_empty() {
                    state.sidebar_tab = state.sidebar_tab.next();
                    state.focus = SurfaceFocus::Sidebar;
                } else if character == '[' && state.composer.is_empty() {
                    state.sidebar_tab = state.sidebar_tab.previous();
                    state.focus = SurfaceFocus::Sidebar;
                } else if character == 't'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.overlay = Some(SurfaceOverlay::Timeline);
                    state.focus = SurfaceFocus::Transcript;
                } else if matches!(state.overlay, Some(SurfaceOverlay::ApprovalPrompt { .. })) {
                    let quick_response = character.to_string();
                    let quick_response_action =
                        crate::conversation::parse_approval_prompt_action_input(
                            quick_response.as_str(),
                        );

                    if quick_response_action.is_some() {
                        let mut cursor = state.composer_cursor;
                        insert_char_at_cursor(&mut state.composer, &mut cursor, character);
                        state.composer_cursor = cursor;
                        state.overlay = None;
                        state.focus = SurfaceFocus::Composer;
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Submit);
                    }
                } else if (character == 'j' || character == 'k')
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    if state.transcript.is_empty() {
                        state.selected_entry = None;
                        state.scroll_offset = 0;
                        state.sticky_bottom = true;
                    } else {
                        let transcript_height = self.transcript_viewport_height_for_state(&state);
                        let current = state
                            .selected_entry
                            .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
                        if character == 'j' {
                            let next_selected = min(
                                current.saturating_add(1),
                                state.transcript.len().saturating_sub(1),
                            );
                            state.selected_entry = Some(next_selected);
                        } else {
                            let next_selected = current.saturating_sub(1);
                            state.selected_entry = Some(next_selected);
                        }
                        if let Some(selected_entry) = state.selected_entry {
                            let aligned_offset = align_scroll_offset_to_selected_entry(
                                &state.transcript,
                                selected_entry,
                                transcript_height,
                                state.scroll_offset,
                            );
                            state.scroll_offset = aligned_offset;
                        }
                        state.sticky_bottom = state.scroll_offset == 0;
                    }
                } else if character == 'g'
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    if state.transcript.is_empty() {
                        state.selected_entry = None;
                        state.scroll_offset = 0;
                        state.sticky_bottom = true;
                    } else {
                        state.selected_entry = Some(0);
                        state.sticky_bottom = false;
                        let transcript_height = self.transcript_viewport_height_for_state(&state);
                        let aligned_offset = align_scroll_offset_to_selected_entry(
                            &state.transcript,
                            0,
                            transcript_height,
                            state.scroll_offset,
                        );
                        state.scroll_offset = aligned_offset;
                    }
                } else if character == 'G'
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    state.selected_entry = state.transcript.len().checked_sub(1);
                    state.scroll_offset = 0;
                    state.sticky_bottom = true;
                } else {
                    let mut cursor = state.composer_cursor;
                    insert_char_at_cursor(&mut state.composer, &mut cursor, character);
                    state.composer_cursor = cursor;
                    state.focus = SurfaceFocus::Composer;
                }
                state.history_index = None;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Unknown
            | Key::UnknownEscSeq(_)
            | Key::Alt
            | Key::Del
            | Key::Shift
            | Key::Insert => Ok(SurfaceLoopAction::Continue),
            _ => Ok(SurfaceLoopAction::Continue),
        }
    }

    fn execute_palette_action(&self, action: CommandPaletteAction) -> CliResult<SurfaceLoopAction> {
        let mut state = self.lock_state();
        state.command_palette = None;
        match action {
            CommandPaletteAction::Help => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_HELP_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Status => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_STATUS_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::History => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_HISTORY_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Compact => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_COMPACT_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Timeline => {
                state.overlay = Some(SurfaceOverlay::Timeline);
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::RenameSession => {
                let initial = state
                    .session_title_override
                    .clone()
                    .unwrap_or_else(|| self.runtime.session_id.clone());
                state.overlay = Some(SurfaceOverlay::InputPrompt {
                    kind: OverlayInputKind::RenameSession,
                    cursor: initial.chars().count(),
                    value: initial,
                });
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::ExportTranscript => {
                let initial = default_export_path(self.runtime.session_id.as_str());
                state.overlay = Some(SurfaceOverlay::InputPrompt {
                    kind: OverlayInputKind::ExportTranscript,
                    cursor: initial.chars().count(),
                    value: initial,
                });
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::JumpLatest => {
                state.sticky_bottom = true;
                state.scroll_offset = 0;
                state.selected_entry = state.transcript.len().checked_sub(1);
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::ToggleSticky => {
                state.sticky_bottom = !state.sticky_bottom;
                if state.sticky_bottom {
                    state.scroll_offset = 0;
                    state.selected_entry = state.transcript.len().checked_sub(1);
                }
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::ToggleSidebar => {
                state.sidebar_visible = !state.sidebar_visible;
                state.focus = if state.sidebar_visible {
                    SurfaceFocus::Sidebar
                } else {
                    SurfaceFocus::Composer
                };
            }
            CommandPaletteAction::CycleSidebarTab => {
                state.sidebar_tab = state.sidebar_tab.next();
                state.focus = SurfaceFocus::Sidebar;
            }
            CommandPaletteAction::ClearComposer => {
                state.composer.clear();
                state.composer_cursor = 0;
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::Exit => return Ok(SurfaceLoopAction::Exit),
        }
        drop(state);
        self.render()?;
        Ok(SurfaceLoopAction::Continue)
    }

    async fn submit_text(&self, text: &str) -> CliResult<SurfaceLoopAction> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(SurfaceLoopAction::Continue);
        }

        if is_exit_command(&self.runtime.config, trimmed) {
            return Ok(SurfaceLoopAction::Exit);
        }

        if trimmed.starts_with('/') {
            {
                let mut state = self.lock_state();
                state.composer.clear();
                state.composer_cursor = 0;
                state.history_index = None;
                state.focus = SurfaceFocus::Composer;
            }
            self.handle_command(trimmed).await?;
            self.render()?;
            return Ok(SurfaceLoopAction::Continue);
        }

        {
            let mut state = self.lock_state();
            state.transcript.push(SurfaceEntry {
                lines: render_cli_chat_message_spec_with_width(
                    &TuiMessageSpec {
                        role: "you".to_owned(),
                        caption: Some("prompt".to_owned()),
                        sections: vec![TuiSectionSpec::Narrative {
                            title: None,
                            lines: vec![trimmed.to_owned()],
                        }],
                        footer_lines: vec!["Enter send · Esc clear · Tab sidebar".to_owned()],
                    },
                    self.content_width(),
                ),
            });
            state.history.push(trimmed.to_owned());
            state.composer.clear();
            state.composer_cursor = 0;
            state.history_index = None;
            state.pending_turn = true;
            state.scroll_offset = 0;
            state.sticky_bottom = true;
            state.selected_entry = Some(state.transcript.len().saturating_sub(1));
            state.focus = SurfaceFocus::Transcript;
        }
        self.render()?;

        let observer = build_surface_live_observer(self.state.clone(), self.term.clone());
        let assistant_text = crate::agent_runtime::AgentRuntime::new()
            .run_turn_with_runtime_and_observer(
                &self.runtime,
                &crate::agent_runtime::AgentTurnRequest {
                    message: trimmed.to_owned(),
                    turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
                    channel_id: self.runtime.session_address.channel_id.clone(),
                    account_id: self.runtime.session_address.account_id.clone(),
                    conversation_id: self.runtime.session_address.conversation_id.clone(),
                    thread_id: self.runtime.session_address.thread_id.clone(),
                    metadata: std::collections::BTreeMap::new(),
                    acp: self.runtime.explicit_acp_request,
                    acp_event_stream: false,
                    acp_bootstrap_mcp_servers: self.runtime.effective_bootstrap_mcp_servers.clone(),
                    acp_cwd: self
                        .runtime
                        .effective_working_directory
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    live_surface_enabled: true,
                },
                None,
                Some(observer),
            )
            .await?
            .output_text;

        {
            let mut state = self.lock_state();
            state.transcript.push(SurfaceEntry {
                lines: render_cli_chat_assistant_lines_with_width(
                    &assistant_text,
                    self.content_width(),
                ),
            });
            if let Some(screen) = build_cli_chat_approval_screen_spec(&assistant_text) {
                state.overlay = Some(SurfaceOverlay::ApprovalPrompt { screen });
            }
            state.pending_turn = false;
            state.live.last_assistant_preview = Some(assistant_text);
            state.live.snapshot = None;
            state.live.state = CliChatLiveSurfaceState::default();
            state.selected_entry = Some(state.transcript.len().saturating_sub(1));
            state.sticky_bottom = true;
        }
        self.render()?;
        Ok(SurfaceLoopAction::Continue)
    }

    async fn handle_command(&self, input: &str) -> CliResult<()> {
        let width = self.content_width();
        let help_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_HELP_COMMAND],
            "usage: /help",
        ))?;

        let status_match = classify_chat_command_match_result(
            operator_surfaces::is_cli_chat_status_command(input),
        )?;

        let compact_match = classify_chat_command_match_result(
            operator_surfaces::is_manual_compaction_command(input),
        )?;

        let history_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_HISTORY_COMMAND],
            "usage: /history",
        ))?;

        let turn_checkpoint_repair_match = classify_chat_command_match_result(
            operator_surfaces::is_turn_checkpoint_repair_command(input),
        )?;

        let lines = match help_match {
            ChatCommandMatchResult::Matched => {
                operator_surfaces::render_cli_chat_help_lines_with_width(width)
            }
            ChatCommandMatchResult::UsageError(usage) => {
                render_cli_chat_command_usage_lines_with_width(&usage, width)
            }
            ChatCommandMatchResult::NotMatched => match status_match {
                ChatCommandMatchResult::Matched => {
                    let summary = operator_surfaces::build_cli_chat_startup_summary(
                        &self.runtime,
                        &self.options,
                    )?;
                    operator_surfaces::render_cli_chat_status_lines_with_width(&summary, width)
                }
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => match compact_match {
                    ChatCommandMatchResult::Matched => {
                        #[cfg(feature = "memory-sqlite")]
                        {
                            let binding =
                                ConversationRuntimeBinding::kernel(&self.runtime.kernel_ctx);
                            let result = operator_surfaces::load_manual_compaction_result(
                                &self.runtime.config,
                                &self.runtime.session_id,
                                &self.runtime.turn_coordinator,
                                binding,
                            )
                            .await?;
                            operator_surfaces::render_manual_compaction_lines_with_width(
                                &self.runtime.session_id,
                                &result,
                                width,
                            )
                        }
                        #[cfg(not(feature = "memory-sqlite"))]
                        {
                            render_cli_chat_feature_unavailable_lines_with_width(
                                "compact",
                                "manual compaction unavailable: memory-sqlite feature disabled",
                                width,
                            )
                        }
                    }
                    ChatCommandMatchResult::UsageError(usage) => {
                        render_cli_chat_command_usage_lines_with_width(&usage, width)
                    }
                    ChatCommandMatchResult::NotMatched => match history_match {
                        ChatCommandMatchResult::Matched => {
                            #[cfg(feature = "memory-sqlite")]
                            {
                                let history_lines = operator_surfaces::load_history_lines(
                                    &self.runtime.session_id,
                                    self.runtime.config.memory.sliding_window,
                                    ConversationRuntimeBinding::kernel(&self.runtime.kernel_ctx),
                                    &self.runtime.memory_config,
                                )
                                .await?;
                                operator_surfaces::render_cli_chat_history_lines_with_width(
                                    &self.runtime.session_id,
                                    self.runtime.config.memory.sliding_window,
                                    &history_lines,
                                    width,
                                )
                            }
                            #[cfg(not(feature = "memory-sqlite"))]
                            {
                                render_cli_chat_feature_unavailable_lines_with_width(
                                    "history",
                                    "history unavailable: memory-sqlite feature disabled",
                                    width,
                                )
                            }
                        }
                        ChatCommandMatchResult::UsageError(usage) => {
                            render_cli_chat_command_usage_lines_with_width(&usage, width)
                        }
                        ChatCommandMatchResult::NotMatched => match turn_checkpoint_repair_match {
                            ChatCommandMatchResult::Matched => {
                                let outcome = self
                                    .runtime
                                    .turn_coordinator
                                    .repair_turn_checkpoint_tail(
                                        &self.runtime.config,
                                        &self.runtime.session_id,
                                        ConversationRuntimeBinding::kernel(
                                            &self.runtime.kernel_ctx,
                                        ),
                                    )
                                    .await?;
                                render_turn_checkpoint_repair_lines_with_width(
                                    &self.runtime.session_id,
                                    &outcome,
                                    width,
                                )
                            }
                            ChatCommandMatchResult::UsageError(usage) => {
                                render_cli_chat_command_usage_lines_with_width(&usage, width)
                            }
                            ChatCommandMatchResult::NotMatched => {
                                render_cli_chat_command_usage_lines_with_width(
                                    "usage: /help | /status | /history | /compact | /turn_checkpoint_repair | /exit",
                                    width,
                                )
                            }
                        },
                    },
                },
            },
        };

        let mut state = self.lock_state();
        state.transcript.push(SurfaceEntry { lines });
        state.selected_entry = Some(state.transcript.len().saturating_sub(1));
        state.sticky_bottom = true;
        state.focus = SurfaceFocus::Transcript;
        Ok(())
    }

    fn submit_input_overlay(&self, kind: OverlayInputKind, value: String) -> CliResult<()> {
        let trimmed = value.trim();
        let mut state = self.lock_state();
        match kind {
            OverlayInputKind::RenameSession => {
                if trimmed.is_empty() {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    return Ok(());
                }
                state.session_title_override = Some(trimmed.to_owned());
                state.transcript.push(SurfaceEntry {
                    lines: render_cli_chat_message_spec_with_width(
                        &TuiMessageSpec {
                            role: "system".to_owned(),
                            caption: Some("session".to_owned()),
                            sections: vec![TuiSectionSpec::Callout {
                                tone: TuiCalloutTone::Success,
                                title: Some("session renamed".to_owned()),
                                lines: vec![format!("Session title updated to `{trimmed}`.")],
                            }],
                            footer_lines: vec![
                                "This rename is local to the current surface.".to_owned(),
                            ],
                        },
                        self.content_width(),
                    ),
                });
            }
            OverlayInputKind::ExportTranscript => {
                if trimmed.is_empty() {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    return Ok(());
                }
                let export_path = PathBuf::from(trimmed);
                ensure_parent_directory_exists(export_path.as_path())?;
                let export_text = format_transcript_export(&state.transcript);
                std::fs::write(export_path.as_path(), export_text).map_err(|error| {
                    let display_path = export_path.display();
                    format!("failed to export transcript to `{display_path}`: {error}")
                })?;
                let exported_path = export_path.display().to_string();
                state.transcript.push(SurfaceEntry {
                    lines: render_cli_chat_message_spec_with_width(
                        &TuiMessageSpec {
                            role: "system".to_owned(),
                            caption: Some("export".to_owned()),
                            sections: vec![TuiSectionSpec::Callout {
                                tone: TuiCalloutTone::Success,
                                title: Some("transcript exported".to_owned()),
                                lines: vec![format!("Saved transcript to `{exported_path}`.")],
                            }],
                            footer_lines: vec![
                                "Use the exported text file for external review or sharing."
                                    .to_owned(),
                            ],
                        },
                        self.content_width(),
                    ),
                });
            }
        }
        state.overlay = None;
        state.focus = SurfaceFocus::Transcript;
        state.selected_entry = Some(state.transcript.len().saturating_sub(1));
        state.sticky_bottom = true;
        Ok(())
    }

    fn render(&self) -> CliResult<()> {
        let (height_u16, width_u16) = self.term.size();
        let total_height = usize::from(height_u16);
        let total_width = usize::from(width_u16);
        let state = self.lock_state().clone();

        let header_lines = crate::presentation::render_compact_brand_header(
            total_width.saturating_sub(2),
            &crate::presentation::BuildVersionInfo::current(),
            Some(session_surface_subtitle(&state)),
        )
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>();

        let sidebar_visible = state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
        let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };
        let content_width = total_width
            .saturating_sub(sidebar_width)
            .saturating_sub(if sidebar_visible { 3 } else { 2 })
            .max(24);

        let reserved_height =
            header_lines.len() + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;
        let transcript_height = total_height.saturating_sub(reserved_height).max(5);
        let transcript_lines =
            self.build_transcript_lines(&state, content_width, transcript_height);
        let sidebar_lines = self.build_sidebar_lines(&state, sidebar_width, transcript_height);
        let composer_lines = self.build_composer_lines(&state, total_width.saturating_sub(2));
        let status_line = self.build_status_line(&state, total_width.saturating_sub(2));
        let overlay_lines =
            self.build_command_palette_lines(&state, total_width, total_height, transcript_height);
        let detail_overlay =
            self.build_entry_detail_overlay_lines(&state, total_width, total_height);
        let timeline_overlay = self.build_timeline_overlay_lines(&state, total_width, total_height);
        let prompt_overlay = self.build_prompt_overlay_lines(&state, total_width, total_height);

        let mut output = String::from(CLEAR_AND_HOME);
        for line in &header_lines {
            output.push_str(line);
            output.push('\n');
        }
        output.push('\n');

        for row in 0..transcript_height {
            let main_line = transcript_lines.get(row).map(String::as_str).unwrap_or("");
            output.push_str(&pad_and_clip(main_line, content_width));
            if sidebar_visible {
                output.push_str(" │ ");
                let side_line = sidebar_lines.get(row).map(String::as_str).unwrap_or("");
                output.push_str(&pad_and_clip(side_line, sidebar_width));
            }
            output.push('\n');
        }

        for line in &composer_lines {
            output.push_str(line);
            output.push('\n');
        }
        output.push_str(&status_line);
        if let Some(overlay) = overlay_lines {
            output.push_str(overlay.as_str());
        }
        if let Some(overlay) = detail_overlay {
            output.push_str(overlay.as_str());
        }
        if let Some(overlay) = timeline_overlay {
            output.push_str(overlay.as_str());
        }
        if let Some(overlay) = prompt_overlay {
            output.push_str(overlay.as_str());
        }

        self.term
            .write_str(output.as_str())
            .map_err(|error| format!("failed to render chat surface: {error}"))?;
        self.term
            .flush()
            .map_err(|error| format!("failed to flush chat surface: {error}"))?;
        Ok(())
    }

    fn build_transcript_lines(
        &self,
        state: &SurfaceState,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for (entry_index, entry) in state.transcript.iter().enumerate() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            for (line_index, line) in entry.lines.iter().enumerate() {
                let clipped = clipped_display_line(line, width.saturating_sub(2));
                if line_index == 0 && state.selected_entry == Some(entry_index) {
                    lines.push(format!("▶ {clipped}"));
                } else {
                    lines.push(clipped);
                }
            }
        }

        if state.pending_turn && !lines.is_empty() {
            lines.push(String::new());
        }

        if state.pending_turn {
            let live_lines = render_cli_chat_live_surface_lines_with_width(
                &state
                    .live
                    .snapshot
                    .clone()
                    .unwrap_or_else(fallback_live_surface_snapshot),
                width,
            );
            lines.extend(
                live_lines
                    .into_iter()
                    .map(|line| clipped_display_line(&line, width)),
            );
        }

        if state.sticky_bottom {
            if lines.len() <= height {
                return lines;
            }

            let start = lines.len().saturating_sub(height);
            return lines.into_iter().skip(start).collect();
        }

        if lines.len() <= height {
            return lines;
        }

        let max_offset = lines.len().saturating_sub(height);
        let scroll_offset = min(state.scroll_offset, max_offset);
        let start = lines.len().saturating_sub(height + scroll_offset);
        lines.into_iter().skip(start).take(height).collect()
    }

    fn build_sidebar_lines(
        &self,
        state: &SurfaceState,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }

        let startup_summary = state
            .startup_summary
            .clone()
            .unwrap_or_else(|| fallback_startup_summary(self.runtime.session_id.as_str()));
        let mut lines = vec![
            format!("right rail · {}", state.sidebar_tab.title()),
            format!("session {}", startup_summary.session_id),
        ];
        lines.push(format!("focus: {}", state.focus.label()));
        let tab_label = format!(
            "tabs: {} | {} | {} | {}",
            if state.sidebar_tab == SidebarTab::Session {
                "[session]"
            } else {
                "session"
            },
            if state.sidebar_tab == SidebarTab::Runtime {
                "[runtime]"
            } else {
                "runtime"
            },
            if state.sidebar_tab == SidebarTab::Tools {
                "[tools]"
            } else {
                "tools"
            },
            if state.sidebar_tab == SidebarTab::Help {
                "[help]"
            } else {
                "help"
            },
        );
        lines.extend(crate::presentation::render_wrapped_display_line(
            &tab_label, width,
        ));
        lines.push(String::new());

        match state.sidebar_tab {
            SidebarTab::Session => {
                lines.push(format!("session: {}", startup_summary.session_id));
                lines.push(format!("config: {}", startup_summary.config_path));
                lines.push(format!("memory: {}", startup_summary.memory_label));
                lines.push(format!("context: {}", startup_summary.context_engine_id));
                lines.push(format!(
                    "context src: {}",
                    startup_summary.context_engine_source
                ));
                lines.push(format!("acp backend: {}", startup_summary.acp_backend_id));
                lines.push(format!("routing: {}", startup_summary.conversation_routing));
                lines.push(format!("sticky: {}", state.sticky_bottom));
                lines.push(format!("entries: {}", state.transcript.len()));
                lines.push(format!(
                    "channels: {}",
                    if startup_summary.allowed_channels.is_empty() {
                        "-".to_owned()
                    } else {
                        startup_summary.allowed_channels.join(", ")
                    }
                ));
            }
            SidebarTab::Runtime => {
                lines.push(format!("acp: {}", startup_summary.acp_enabled));
                lines.push(format!("dispatch: {}", startup_summary.dispatch_enabled));
                lines.push(format!(
                    "event stream: {}",
                    startup_summary.event_stream_enabled
                ));
                let working_directory = startup_summary
                    .working_directory
                    .unwrap_or_else(|| "-".to_owned());
                lines.push(format!("cwd: {}", working_directory));
                lines.push(format!(
                    "phase: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.phase.as_str())
                        .unwrap_or("idle")
                ));
                lines.push(format!(
                    "round: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.provider_round)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "messages: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.message_count)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "tokens: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.estimated_tokens)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
            }
            SidebarTab::Tools => {
                lines.push(format!(
                    "tool calls: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.tool_call_count)
                        .unwrap_or(0)
                ));
                let tool_lines = state
                    .live
                    .snapshot
                    .as_ref()
                    .map(|snapshot| {
                        format_cli_chat_live_tool_activity_lines(snapshot.tools.as_slice())
                    })
                    .unwrap_or_default();
                if tool_lines.is_empty() {
                    lines.push("no tool activity recorded".to_owned());
                } else {
                    lines.extend(tool_lines.into_iter().take(10));
                }
            }
            SidebarTab::Help => {
                lines.push("shortcuts".to_owned());
                lines.push("Enter send".to_owned());
                lines.push("Esc clear / exit".to_owned());
                lines.push("Tab cycle focus".to_owned());
                lines.push("[ ] / Home End switch rail tab".to_owned());
                lines.push("PgUp / PgDn transcript scroll".to_owned());
                lines.push("j / k transcript move".to_owned());
                lines.push("Enter on transcript → detail".to_owned());
                lines.push("g / G transcript jump".to_owned());
                lines.push("t timeline overlay".to_owned());
                lines.push("← / → / Home / End composer cursor".to_owned());
                lines.push("↑ / ↓ composer multiline move".to_owned());
                lines.push(": command palette".to_owned());
                lines.push("/help /status /history /compact".to_owned());
            }
        }

        if let Some(preview) = state.live.last_assistant_preview.as_deref() {
            lines.push(String::new());
            lines.push("last reply".to_owned());
            lines.extend(
                crate::presentation::render_wrapped_display_line(preview, width)
                    .into_iter()
                    .take(8),
            );
        }

        if let Some(selected) = state.selected_entry
            && let Some(entry) = state.transcript.get(selected)
        {
            lines.push(String::new());
            lines.push(format!("selected entry: {}", selected + 1));
            lines.extend(
                entry
                    .lines
                    .iter()
                    .flat_map(|line| crate::presentation::render_wrapped_display_line(line, width))
                    .take(6),
            );
        }

        lines.truncate(height);
        lines
    }

    fn build_composer_lines(&self, state: &SurfaceState, width: usize) -> Vec<String> {
        let draft_lines = composer_display_lines(
            &composer_text_with_cursor(&state.composer, state.composer_cursor),
            width.saturating_sub(2),
            2,
        );
        let prompt_line = if state.composer.is_empty() {
            format!("╭─ compose · focus={}", state.focus.label())
        } else {
            format!(
                "╭─ compose · {} chars · focus={}",
                state.composer.chars().count(),
                state.focus.label()
            )
        };
        let body_line = format!("│ {}", draft_lines.first().cloned().unwrap_or_default());
        let second_line = if draft_lines.len() > 1 {
            format!("│ {}", draft_lines.get(1).cloned().unwrap_or_default())
        } else if let Some(hint) = slash_command_hint(&state.composer) {
            format!("│ {hint}")
        } else {
            "│".to_owned()
        };
        let hint = if state.command_palette.is_some() {
            "╰─ command palette active · type filter · ↑↓ choose · Enter run · Esc close"
        } else if state.composer.starts_with('/') {
            "╰─ slash mode · Enter send command · : open command palette"
        } else if should_continue_multiline(&state.composer) {
            "╰─ multiline compose · trailing \\ inserts newline on Enter"
        } else {
            "╰─ Enter send · : command palette · /help for commands"
        };
        vec![prompt_line, body_line, second_line, hint.to_owned()]
    }

    fn build_status_line(&self, state: &SurfaceState, width: usize) -> String {
        let mut status = format!(
            "{} · focus={} · rail={} · entries={} · scroll={} · sticky={} · overlay={}",
            state.footer_notice,
            state.focus.label(),
            state.sidebar_tab.title(),
            state.transcript.len(),
            state.scroll_offset,
            state.sticky_bottom,
            current_overlay_label(state)
        );
        if state.pending_turn {
            status.push_str(" · turn running");
        }
        clipped_display_line(&status, width)
    }

    fn build_command_palette_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        _total_height: usize,
        transcript_height: usize,
    ) -> Option<String> {
        let palette = state.command_palette.as_ref()?;
        let filtered_items = filtered_command_palette_items(&palette.query);
        let overlay_width = COMMAND_OVERLAY_WIDTH
            .min(total_width.saturating_sub(4))
            .max(24);
        let x = total_width.saturating_sub(overlay_width + 2);
        let y = transcript_height.saturating_sub(8).max(2);
        let header = if palette.query.is_empty() {
            "╭─ commands".to_owned()
        } else {
            format!("╭─ commands · query={}", palette.query)
        };
        let mut lines = vec![format!("\x1b[{};{}H{}", y + 1, x + 1, header)];
        for (index, (label, detail, _)) in filtered_items.iter().enumerate() {
            let marker = if index == palette.selected { '>' } else { ' ' };
            let row = y + 2 + index;
            lines.push(format!(
                "\x1b[{};{}H│ {} {}",
                row + 1,
                x + 1,
                marker,
                pad_and_clip(label, overlay_width.saturating_sub(4))
            ));
            let detail_row = row + 1;
            lines.push(format!(
                "\x1b[{};{}H│   {}",
                detail_row + 1,
                x + 1,
                pad_and_clip(detail, overlay_width.saturating_sub(4))
            ));
        }
        if filtered_items.is_empty() {
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2,
                x + 1,
                pad_and_clip(
                    "no commands match the current query",
                    overlay_width.saturating_sub(4)
                )
            ));
        }
        let bottom_row = y + 2 + filtered_items.len().max(1) * 2;
        lines.push(format!(
            "\x1b[{};{}H╰─ type to filter · Enter run · Esc close",
            bottom_row + 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    fn build_entry_detail_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        let SurfaceOverlay::EntryDetails { entry_index } = state.overlay.as_ref()?.clone() else {
            return None;
        };
        let entry = state.transcript.get(entry_index)?;
        let overlay_width = min(total_width.saturating_sub(6), 80).max(28);
        let overlay_height = min(total_height.saturating_sub(6), 18).max(8);
        let x = (total_width.saturating_sub(overlay_width)) / 2;
        let y = (total_height.saturating_sub(overlay_height)) / 2;
        let mut lines = vec![format!(
            "\x1b[{};{}H╭─ entry details · #{}",
            y + 1,
            x + 1,
            entry_index + 1
        )];

        let body_width = overlay_width.saturating_sub(4);
        let mut rendered = Vec::new();
        for line in &entry.lines {
            let wrapped = crate::presentation::render_wrapped_display_line(line, body_width);
            if wrapped.is_empty() {
                rendered.push(String::new());
            } else {
                rendered.extend(wrapped);
            }
        }

        let visible_rows = overlay_height.saturating_sub(3);
        for row in 0..visible_rows {
            let rendered_line = rendered.get(row).cloned().unwrap_or_default();
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2 + row,
                x + 1,
                pad_and_clip(rendered_line.as_str(), body_width)
            ));
        }
        lines.push(format!(
            "\x1b[{};{}H╰─ Esc close · j/k move · g/G jump",
            y + overlay_height - 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    fn build_timeline_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        if !matches!(state.overlay, Some(SurfaceOverlay::Timeline)) {
            return None;
        }
        let overlay_width = min(total_width.saturating_sub(8), 72).max(32);
        let overlay_height = min(total_height.saturating_sub(8), 18).max(8);
        let x = (total_width.saturating_sub(overlay_width)) / 2;
        let y = (total_height.saturating_sub(overlay_height)) / 2;
        let mut lines = vec![format!("\x1b[{};{}H╭─ timeline", y + 1, x + 1)];
        let body_rows = overlay_height.saturating_sub(3);
        let selected = state
            .selected_entry
            .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
        let start_index = selected.saturating_sub(body_rows / 2);

        for row in 0..body_rows {
            let entry_index = start_index.saturating_add(row);
            let label = if let Some(entry) = state.transcript.get(entry_index) {
                let title = entry
                    .lines
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "(empty entry)".to_owned());
                let prefix = if entry_index == selected { '>' } else { ' ' };
                format!("{prefix} {:>3}. {}", entry_index + 1, title)
            } else {
                String::new()
            };
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2 + row,
                x + 1,
                pad_and_clip(label.as_str(), overlay_width.saturating_sub(4))
            ));
        }
        lines.push(format!(
            "\x1b[{};{}H╰─ j/k move · Enter open · Esc close",
            y + overlay_height - 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    fn build_prompt_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        match state.overlay.as_ref()? {
            SurfaceOverlay::ConfirmExit => {
                let overlay_width = min(total_width.saturating_sub(12), 56).max(28);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(6)) / 2;
                Some(
                    [
                        format!("\x1b[{};{}H╭─ confirm exit", y + 1, x + 1),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 2,
                            x + 1,
                            pad_and_clip(
                                "Press Enter to leave the session surface, or Esc to continue.",
                                overlay_width.saturating_sub(4),
                            )
                        ),
                        format!("\x1b[{};{}H╰─ Enter confirm · Esc cancel", y + 3, x + 1),
                    ]
                    .join(""),
                )
            }
            SurfaceOverlay::InputPrompt {
                kind,
                value,
                cursor,
            } => {
                let overlay_width = min(total_width.saturating_sub(10), 72).max(32);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(8)) / 2;
                let title = match kind {
                    OverlayInputKind::RenameSession => "rename session",
                    OverlayInputKind::ExportTranscript => "export transcript",
                };
                let hint = match kind {
                    OverlayInputKind::RenameSession => {
                        "Set a local session title for this fullscreen surface."
                    }
                    OverlayInputKind::ExportTranscript => {
                        "Choose a file path to write the current transcript."
                    }
                };
                let input = composer_text_with_cursor(value, *cursor);
                Some(
                    [
                        format!("\x1b[{};{}H╭─ {}", y + 1, x + 1, title),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 2,
                            x + 1,
                            pad_and_clip(hint, overlay_width.saturating_sub(4))
                        ),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 3,
                            x + 1,
                            pad_and_clip(input.as_str(), overlay_width.saturating_sub(4))
                        ),
                        format!("\x1b[{};{}H╰─ Enter save · Esc cancel", y + 4, x + 1),
                    ]
                    .join(""),
                )
            }
            SurfaceOverlay::ApprovalPrompt { screen } => {
                let overlay_width = min(total_width.saturating_sub(10), 88).max(36);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(14)) / 2;
                let lines = render_tui_screen_spec(screen, overlay_width.saturating_sub(4), false);
                let mut rendered = vec![format!("\x1b[{};{}H╭─ approval required", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(10).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Type approval response in composer · Esc close",
                    y + 12,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::EntryDetails { .. } | SurfaceOverlay::Timeline => None,
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, SurfaceState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        }
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
            Some(session_surface_subtitle(state)),
        );
        let header_height = header_lines.len();
        let reserved_height = header_height + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;

        total_height.saturating_sub(reserved_height).max(5)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SurfaceLoopAction {
    Continue,
    Submit,
    RunCommand(String),
    Exit,
}

struct SurfaceLiveObserver {
    state: Arc<Mutex<SurfaceState>>,
    term: Term,
}

fn live_surface_content_width(term: &Term, state: &SurfaceState) -> usize {
    let (_, width_u16) = term.size();
    let total_width = usize::from(width_u16);
    let sidebar_visible = state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
    let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };

    total_width
        .saturating_sub(sidebar_width)
        .saturating_sub(if sidebar_visible { 3 } else { 2 })
        .max(24)
}

impl ConversationTurnObserver for SurfaceLiveObserver {
    fn on_phase(&self, event: ConversationTurnPhaseEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };

        if cli_chat_live_phase_starts_provider_request(event.phase) {
            reset_cli_chat_live_request_state(&mut state.live.state);
        }

        state.live.state.latest_phase_event = Some(event.clone());
        reconcile_cli_chat_live_tool_states_for_phase(
            &mut state.live.state.tool_states,
            event.phase,
        );
        sync_live_surface_snapshot(&mut state.live);
        state.live.last_phase_label = event.phase.as_str().to_owned();
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_tool(&self, event: ConversationTurnToolEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);

        apply_cli_chat_live_tool_event(&mut state.live.state, &event, render_width);
        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_runtime(&self, event: ConversationTurnRuntimeEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);

        apply_cli_chat_live_runtime_event(&mut state.live.state, &event, render_width);
        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }

    fn on_streaming_token(&self, event: crate::acp::StreamingTokenEvent) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned_state) => poisoned_state.into_inner(),
        };
        let render_width = live_surface_content_width(&self.term, &state);
        let current_phase = state
            .live
            .state
            .latest_phase_event
            .as_ref()
            .map(|phase_event| phase_event.phase);

        if let Some(text_delta) = event.delta.text
            && let Some(current_phase) = current_phase
            && phase_supports_cli_chat_live_preview(current_phase)
        {
            let preview_char_limit = cli_chat_live_preview_char_limit(render_width);
            state.live.state.total_text_chars_seen = state
                .live
                .state
                .total_text_chars_seen
                .saturating_add(text_delta.chars().count());
            append_cli_chat_live_buffer(
                &mut state.live.state.draft_preview,
                text_delta.as_str(),
                preview_char_limit,
            );
        }

        let tool_call_update = match (event.delta.tool_call, event.index) {
            (Some(tool_call_delta), Some(index)) => Some((tool_call_delta, index)),
            (Some(_), None) | (None, Some(_)) | (None, None) => None,
        };

        if let Some((tool_call_delta, index)) = tool_call_update {
            update_cli_chat_live_tool_state(
                &mut state.live.state,
                index,
                &tool_call_delta,
                render_width,
            );
        }

        sync_live_surface_snapshot(&mut state.live);
        drop(state);
        let _ = render_live_update(self.term.clone(), self.state.clone());
    }
}

fn build_surface_live_observer(
    state: Arc<Mutex<SurfaceState>>,
    term: Term,
) -> ConversationTurnObserverHandle {
    Arc::new(SurfaceLiveObserver { state, term })
}

fn render_live_update(term: Term, state: Arc<Mutex<SurfaceState>>) -> CliResult<()> {
    let snapshot_state = match state.lock() {
        Ok(state) => state.clone(),
        Err(poisoned_state) => poisoned_state.into_inner().clone(),
    };
    let (height_u16, width_u16) = term.size();
    let total_height = usize::from(height_u16);
    let total_width = usize::from(width_u16);
    let header_lines = crate::presentation::render_compact_brand_header(
        total_width.saturating_sub(2),
        &crate::presentation::BuildVersionInfo::current(),
        Some(session_surface_subtitle(&snapshot_state)),
    )
    .into_iter()
    .map(|line| line.text)
    .collect::<Vec<_>>();
    let sidebar_visible = snapshot_state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
    let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };
    let content_width = total_width
        .saturating_sub(sidebar_width)
        .saturating_sub(if sidebar_visible { 3 } else { 2 })
        .max(24);
    let reserved_height = header_lines.len() + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;
    let transcript_height = total_height.saturating_sub(reserved_height).max(5);
    let transcript_lines = {
        let mut lines = Vec::new();
        for (entry_index, entry) in snapshot_state.transcript.iter().enumerate() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            for (line_index, line) in entry.lines.iter().enumerate() {
                let clipped = clipped_display_line(line, content_width.saturating_sub(2));
                if line_index == 0 && snapshot_state.selected_entry == Some(entry_index) {
                    lines.push(format!("▶ {clipped}"));
                } else {
                    lines.push(clipped);
                }
            }
        }
        if snapshot_state.pending_turn {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(
                render_cli_chat_live_surface_lines_with_width(
                    &snapshot_state
                        .live
                        .snapshot
                        .clone()
                        .unwrap_or_else(fallback_live_surface_snapshot),
                    content_width,
                )
                .into_iter()
                .map(|line| clipped_display_line(&line, content_width)),
            );
        }
        if lines.len() > transcript_height {
            let start = lines.len().saturating_sub(transcript_height);
            lines.into_iter().skip(start).collect()
        } else {
            lines
        }
    };

    let startup_summary = snapshot_state
        .startup_summary
        .clone()
        .unwrap_or_else(|| fallback_startup_summary("default"));
    let mut sidebar_lines = vec![
        format!("right rail · {}", snapshot_state.sidebar_tab.title()),
        format!("session {}", startup_summary.session_id),
        format!("focus: {}", snapshot_state.focus.label()),
        format!("sticky: {}", snapshot_state.sticky_bottom),
    ];
    if let Some(preview) = snapshot_state.live.last_assistant_preview.as_deref() {
        sidebar_lines.push(String::new());
        sidebar_lines.push("last reply".to_owned());
        sidebar_lines.extend(
            crate::presentation::render_wrapped_display_line(preview, sidebar_width)
                .into_iter()
                .take(8),
        );
    }

    let draft_lines = composer_display_lines(
        &composer_text_with_cursor(&snapshot_state.composer, snapshot_state.composer_cursor),
        total_width.saturating_sub(4),
        2,
    );
    let composer_lines = vec![
        format!("╭─ compose · focus={}", snapshot_state.focus.label()),
        format!("│ {}", draft_lines.first().cloned().unwrap_or_default()),
        if draft_lines.len() > 1 {
            format!("│ {}", draft_lines.get(1).cloned().unwrap_or_default())
        } else if let Some(hint) = slash_command_hint(&snapshot_state.composer) {
            format!("│ {hint}")
        } else {
            "│ turn running…".to_owned()
        },
        "╰─ Enter send · : command palette · /help for commands".to_owned(),
    ];
    let mut status_text = format!(
        "Esc clear · ↑↓ history/scroll · PgUp/PgDn transcript · Tab focus · : commands · focus={} · rail={} · sticky={}",
        snapshot_state.focus.label(),
        snapshot_state.sidebar_tab.title(),
        snapshot_state.sticky_bottom
    );
    if snapshot_state.pending_turn {
        status_text.push_str(" · turn running");
    }
    let status_line = clipped_display_line(&status_text, total_width.saturating_sub(2));
    let mut output = String::from(CLEAR_AND_HOME);
    for line in header_lines {
        output.push_str(&line);
        output.push('\n');
    }
    output.push('\n');
    for row in 0..transcript_height {
        let line = transcript_lines.get(row).map(String::as_str).unwrap_or("");
        output.push_str(&pad_and_clip(line, content_width));
        if sidebar_visible {
            output.push_str(" │ ");
            let side_line = sidebar_lines.get(row).map(String::as_str).unwrap_or("");
            output.push_str(&pad_and_clip(side_line, sidebar_width));
        }
        output.push('\n');
    }
    for line in composer_lines {
        output.push_str(&line);
        output.push('\n');
    }
    output.push_str(&status_line);
    term.write_str(output.as_str())
        .map_err(|error| format!("failed to refresh live surface: {error}"))?;
    term.flush()
        .map_err(|error| format!("failed to flush live surface: {error}"))?;
    Ok(())
}

fn pad_and_clip(line: &str, width: usize) -> String {
    let clipped = clipped_display_line(line, width);
    let clipped_len = clipped.chars().count();
    if clipped_len >= width {
        return clipped;
    }
    format!("{clipped}{}", " ".repeat(width - clipped_len))
}

fn clipped_display_line(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let char_count = line.chars().count();
    if char_count <= width {
        return line.to_owned();
    }
    let mut result = line
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

fn fallback_startup_summary(session_id: &str) -> operator_surfaces::CliChatStartupSummary {
    operator_surfaces::CliChatStartupSummary {
        config_path: "-".to_owned(),
        memory_label: "-".to_owned(),
        session_id: session_id.to_owned(),
        context_engine_id: "-".to_owned(),
        context_engine_source: "-".to_owned(),
        compaction_enabled: false,
        compaction_min_messages: None,
        compaction_trigger_estimated_tokens: None,
        compaction_preserve_recent_turns: 0,
        compaction_fail_open: false,
        acp_enabled: false,
        dispatch_enabled: false,
        conversation_routing: "-".to_owned(),
        allowed_channels: Vec::new(),
        acp_backend_id: "-".to_owned(),
        acp_backend_source: "-".to_owned(),
        explicit_acp_request: false,
        event_stream_enabled: false,
        bootstrap_mcp_servers: Vec::new(),
        working_directory: None,
    }
}

fn session_surface_subtitle(state: &SurfaceState) -> &str {
    state
        .session_title_override
        .as_deref()
        .unwrap_or("interactive session surface")
}

fn default_export_path(session_id: &str) -> String {
    let sanitized_session_id = sanitize_session_id_for_export(session_id);
    let file_name = format!("loongclaw-{sanitized_session_id}-transcript.txt");
    let exports_dir = loongclaw_exports_dir();
    let export_path = exports_dir.join(file_name);

    export_path.display().to_string()
}

fn sanitize_session_id_for_export(session_id: &str) -> String {
    session_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                return character;
            }

            '_'
        })
        .collect()
}

fn loongclaw_exports_dir() -> PathBuf {
    let loongclaw_home = crate::config::default_loongclaw_home();
    loongclaw_home.join("exports")
}

fn ensure_parent_directory_exists(path: &Path) -> CliResult<()> {
    let Some(parent_dir) = path.parent() else {
        return Ok(());
    };

    if parent_dir.as_os_str().is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(parent_dir).map_err(|error| {
        let display_path = parent_dir.display();
        format!("failed to create transcript export directory `{display_path}`: {error}")
    })
}

fn format_transcript_export(entries: &[SurfaceEntry]) -> String {
    let mut rendered = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        rendered.push(format!("## Entry {}", index + 1));
        rendered.extend(entry.lines.iter().cloned());
        rendered.push(String::new());
    }
    rendered.join("\n")
}

fn current_overlay_label(state: &SurfaceState) -> &'static str {
    match state.overlay.as_ref() {
        Some(SurfaceOverlay::EntryDetails { .. }) => "entry",
        Some(SurfaceOverlay::Timeline) => "timeline",
        Some(SurfaceOverlay::ConfirmExit) => "confirm-exit",
        Some(SurfaceOverlay::InputPrompt { kind, .. }) => match kind {
            OverlayInputKind::RenameSession => "rename",
            OverlayInputKind::ExportTranscript => "export",
        },
        Some(SurfaceOverlay::ApprovalPrompt { .. }) => "approval",
        None => "none",
    }
}

fn composer_display_lines(value: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut wrapped = crate::presentation::render_wrapped_display_line(value, width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    if wrapped.len() > max_lines {
        wrapped.truncate(max_lines);
    }
    wrapped
}

fn composer_text_with_cursor(value: &str, cursor: usize) -> String {
    let mut rendered = String::new();
    let mut inserted = false;
    for (index, character) in value.chars().enumerate() {
        if index == cursor {
            rendered.push('▏');
            inserted = true;
        }
        rendered.push(character);
    }
    if !inserted {
        rendered.push('▏');
    }
    rendered
}

fn insert_char_at_cursor(value: &mut String, cursor: &mut usize, character: char) {
    let mut chars = value.chars().collect::<Vec<_>>();
    let insert_at = min(*cursor, chars.len());
    chars.insert(insert_at, character);
    *value = chars.into_iter().collect();
    *cursor = insert_at.saturating_add(1);
}

fn remove_char_before_cursor(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let mut chars = value.chars().collect::<Vec<_>>();
    let remove_at = cursor.saturating_sub(1);
    if remove_at < chars.len() {
        chars.remove(remove_at);
        *value = chars.into_iter().collect();
        *cursor = remove_at;
    }
}

fn move_cursor_vertically(value: &str, cursor: usize, direction: isize) -> usize {
    let chars = value.chars().collect::<Vec<_>>();
    let cursor = min(cursor, chars.len());
    let mut current_line_start = 0;
    let mut index = 0;
    while index < cursor {
        if chars.get(index).is_some_and(|character| *character == '\n') {
            current_line_start = index.saturating_add(1);
        }
        index = index.saturating_add(1);
    }
    let current_column = cursor.saturating_sub(current_line_start);
    let mut current_line_end = chars.len();
    let mut forward_index = cursor;
    while forward_index < chars.len() {
        if chars
            .get(forward_index)
            .is_some_and(|character| *character == '\n')
        {
            current_line_end = forward_index;
            break;
        }
        forward_index = forward_index.saturating_add(1);
    }

    if direction < 0 {
        if current_line_start == 0 {
            return cursor;
        }
        let prev_line_end = current_line_start.saturating_sub(1);
        let mut prev_line_start = 0;
        let mut reverse_index = 0;
        while reverse_index < prev_line_end {
            if chars
                .get(reverse_index)
                .is_some_and(|character| *character == '\n')
            {
                prev_line_start = reverse_index.saturating_add(1);
            }
            reverse_index = reverse_index.saturating_add(1);
        }
        let prev_len = prev_line_end.saturating_sub(prev_line_start);
        return prev_line_start + min(current_column, prev_len);
    }

    if current_line_end >= chars.len() {
        return cursor;
    }
    let next_line_start = current_line_end.saturating_add(1);
    let mut next_line_end = chars.len();
    let mut next_index = next_line_start;
    while next_index < chars.len() {
        if chars
            .get(next_index)
            .is_some_and(|character| *character == '\n')
        {
            next_line_end = next_index;
            break;
        }
        next_index = next_index.saturating_add(1);
    }
    let next_len = next_line_end.saturating_sub(next_line_start);
    next_line_start + min(current_column, next_len)
}

fn slash_command_hint(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let known = [
        CLI_CHAT_HELP_COMMAND,
        CLI_CHAT_STATUS_COMMAND,
        CLI_CHAT_HISTORY_COMMAND,
        CLI_CHAT_COMPACT_COMMAND,
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND,
    ];
    let matches = known
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Some("unknown slash command".to_owned());
    }

    Some(format!("matches: {}", matches.join(" · ")))
}

fn should_continue_multiline(value: &str) -> bool {
    value.ends_with('\\')
}

fn should_continue_multiline_at_cursor(value: &str, cursor: usize) -> bool {
    let total_chars = value.chars().count();
    if cursor != total_chars {
        return false;
    }

    should_continue_multiline(value)
}

fn flattened_entry_line_ranges(entries: &[SurfaceEntry]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut next_line_index: usize = 0;

    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index > 0 {
            next_line_index = next_line_index.saturating_add(1);
        }

        let line_count = entry.lines.len().max(1);
        let start_line_index = next_line_index;
        let end_line_index = start_line_index.saturating_add(line_count);
        let entry_range = start_line_index..end_line_index;

        ranges.push(entry_range);
        next_line_index = end_line_index;
    }

    ranges
}

fn viewport_start_for_scroll_offset(
    total_lines: usize,
    viewport_height: usize,
    scroll_offset: usize,
) -> usize {
    if total_lines <= viewport_height {
        return 0;
    }

    let max_scroll_offset = total_lines.saturating_sub(viewport_height);
    let clamped_scroll_offset = min(scroll_offset, max_scroll_offset);
    total_lines.saturating_sub(viewport_height.saturating_add(clamped_scroll_offset))
}

fn scroll_offset_for_viewport_start(
    total_lines: usize,
    viewport_height: usize,
    viewport_start: usize,
) -> usize {
    if total_lines <= viewport_height {
        return 0;
    }

    let max_viewport_start = total_lines.saturating_sub(viewport_height);
    let clamped_viewport_start = min(viewport_start, max_viewport_start);
    total_lines.saturating_sub(viewport_height.saturating_add(clamped_viewport_start))
}

fn align_scroll_offset_to_selected_entry(
    entries: &[SurfaceEntry],
    selected_entry: usize,
    viewport_height: usize,
    scroll_offset: usize,
) -> usize {
    let entry_ranges = flattened_entry_line_ranges(entries);
    let Some(selected_range) = entry_ranges.get(selected_entry) else {
        return scroll_offset;
    };

    let total_lines = entry_ranges.last().map(|range| range.end).unwrap_or(0);

    if total_lines <= viewport_height {
        return 0;
    }

    let viewport_start =
        viewport_start_for_scroll_offset(total_lines, viewport_height, scroll_offset);
    let viewport_end = viewport_start.saturating_add(viewport_height);

    if selected_range.start < viewport_start {
        return scroll_offset_for_viewport_start(
            total_lines,
            viewport_height,
            selected_range.start,
        );
    }

    if selected_range.end > viewport_end {
        let next_viewport_start = selected_range.end.saturating_sub(viewport_height);

        return scroll_offset_for_viewport_start(total_lines, viewport_height, next_viewport_start);
    }

    scroll_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_tab_cycles_forward_and_backward() {
        assert_eq!(SidebarTab::Session.next(), SidebarTab::Runtime);
        assert_eq!(SidebarTab::Runtime.next(), SidebarTab::Tools);
        assert_eq!(SidebarTab::Tools.next(), SidebarTab::Help);
        assert_eq!(SidebarTab::Help.next(), SidebarTab::Session);
        assert_eq!(SidebarTab::Session.previous(), SidebarTab::Help);
    }

    #[test]
    fn clipped_display_line_adds_ellipsis_when_needed() {
        assert_eq!(clipped_display_line("abcdef", 4), "abc…");
        assert_eq!(clipped_display_line("abc", 4), "abc");
    }

    #[test]
    fn composer_display_lines_wraps_and_limits_rows() {
        let lines = composer_display_lines("alpha beta gamma delta", 10, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("alpha"));
    }

    #[test]
    fn command_palette_items_have_stable_default_selection() {
        let palette = CommandPaletteState::default();
        let items = CommandPaletteAction::items();
        assert_eq!(palette.selected, 0);
        assert_eq!(palette.query, "");
        assert_eq!(items[0].0, "Help");
        assert!(items.iter().any(|item| item.0 == "Jump to latest"));
    }

    #[test]
    fn surface_focus_cycles_without_palette() {
        assert_eq!(
            SurfaceFocus::Composer.next(true, false),
            SurfaceFocus::Transcript
        );
        assert_eq!(
            SurfaceFocus::Transcript.next(true, false),
            SurfaceFocus::Sidebar
        );
        assert_eq!(
            SurfaceFocus::Sidebar.next(true, false),
            SurfaceFocus::Composer
        );
    }

    #[test]
    fn slash_command_hint_surfaces_matches() {
        let hint = slash_command_hint("/hi").expect("hint");
        assert!(hint.contains("/history"));
        assert!(slash_command_hint("hello").is_none());
    }

    #[test]
    fn should_continue_multiline_detects_trailing_backslash() {
        assert!(should_continue_multiline("hello\\"));
        assert!(!should_continue_multiline("hello"));
    }

    #[test]
    fn should_continue_multiline_at_cursor_requires_cursor_at_end() {
        assert!(should_continue_multiline_at_cursor("hello\\", 6));
        assert!(!should_continue_multiline_at_cursor("hello\\", 3));
        assert!(!should_continue_multiline_at_cursor("hello", 5));
    }

    #[test]
    fn terminal_surface_allowed_requires_interactive_stdin_and_stdout() {
        assert!(terminal_surface_allowed(true, true));
        assert!(!terminal_surface_allowed(true, false));
        assert!(!terminal_surface_allowed(false, true));
        assert!(!terminal_surface_allowed(false, false));
    }

    #[test]
    fn composer_text_with_cursor_inserts_marker() {
        assert_eq!(composer_text_with_cursor("abc", 1), "a▏bc");
        assert_eq!(composer_text_with_cursor("", 0), "▏");
    }

    #[test]
    fn insert_and_remove_char_at_cursor_updates_cursor_position() {
        let mut value = "ac".to_owned();
        let mut cursor = 1;
        insert_char_at_cursor(&mut value, &mut cursor, 'b');
        assert_eq!(value, "abc");
        assert_eq!(cursor, 2);
        remove_char_before_cursor(&mut value, &mut cursor);
        assert_eq!(value, "ac");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn move_cursor_vertically_preserves_column_when_possible() {
        let value = "abc\ndefg\nxy";
        assert_eq!(move_cursor_vertically(value, 5, -1), 1);
        assert_eq!(move_cursor_vertically(value, 1, 1), 5);
        assert_eq!(move_cursor_vertically(value, 7, 1), 11);
    }

    #[test]
    fn command_palette_items_include_jump_and_sticky_actions() {
        let labels = CommandPaletteAction::items()
            .iter()
            .map(|item| item.0)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Jump to latest"));
        assert!(labels.contains(&"Toggle sticky scroll"));
        assert!(labels.contains(&"Timeline"));
    }

    #[test]
    fn filtered_command_palette_items_respects_query() {
        let filtered = filtered_command_palette_items("time");
        assert!(filtered.iter().any(|item| item.0 == "Timeline"));
        assert!(!filtered.iter().any(|item| item.0 == "Compact"));
    }

    #[test]
    fn current_overlay_label_reports_overlay_kind() {
        let mut state = SurfaceState {
            startup_summary: None,
            session_title_override: None,
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
            overlay: None,
            live: LiveSurfaceModel::default(),
            footer_notice: String::new(),
            pending_turn: false,
        };
        assert_eq!(current_overlay_label(&state), "none");
        state.overlay = Some(SurfaceOverlay::Timeline);
        assert_eq!(current_overlay_label(&state), "timeline");
    }

    #[test]
    fn align_scroll_offset_to_selected_entry_keeps_entry_visible() {
        let entries = vec![
            SurfaceEntry {
                lines: vec!["entry 1".to_owned()],
            },
            SurfaceEntry {
                lines: vec!["entry 2".to_owned(), "entry 2 detail".to_owned()],
            },
            SurfaceEntry {
                lines: vec!["entry 3".to_owned()],
            },
        ];
        let viewport_height = 2;
        let current_scroll_offset = 0;
        let aligned_offset = align_scroll_offset_to_selected_entry(
            &entries,
            1,
            viewport_height,
            current_scroll_offset,
        );

        assert_eq!(aligned_offset, 2);
    }

    #[test]
    fn default_export_path_uses_loongclaw_exports_directory() {
        let export_path = PathBuf::from(default_export_path("session:/bad"));
        let file_name = export_path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("export file name");
        let parent_dir = export_path
            .parent()
            .and_then(|value| value.file_name())
            .and_then(|value| value.to_str())
            .expect("export parent directory");

        assert_eq!(parent_dir, "exports");
        assert_eq!(file_name, "loongclaw-session__bad-transcript.txt");
    }

    #[test]
    fn ensure_parent_directory_exists_ignores_relative_files_without_parent() {
        let path = Path::new("transcript.txt");
        let result = ensure_parent_directory_exists(path);

        assert!(result.is_ok());
    }
}
