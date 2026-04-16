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
    active_provider_label: String,
    session_title_override: Option<String>,
    last_approval: Option<ApprovalSurfaceSummary>,
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
    Mission,
    Workers,
    Review,
    Help,
}

impl SidebarTab {
    fn title(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Runtime => "runtime",
            Self::Tools => "tools",
            Self::Mission => "mission",
            Self::Workers => "workers",
            Self::Review => "review",
            Self::Help => "help",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Session => Self::Runtime,
            Self::Runtime => Self::Tools,
            Self::Tools => Self::Mission,
            Self::Mission => Self::Workers,
            Self::Workers => Self::Review,
            Self::Review => Self::Help,
            Self::Help => Self::Session,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Session => Self::Help,
            Self::Runtime => Self::Session,
            Self::Tools => Self::Runtime,
            Self::Mission => Self::Tools,
            Self::Workers => Self::Mission,
            Self::Review => Self::Workers,
            Self::Help => Self::Review,
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
    Welcome {
        screen: TuiScreenSpec,
    },
    SessionQueue {
        selected: usize,
        items: Vec<SessionQueueItemSummary>,
    },
    SessionDetails {
        title: String,
        lines: Vec<String>,
    },
    ReviewQueue {
        selected: usize,
        items: Vec<ApprovalQueueItemSummary>,
    },
    MissionControl {
        lines: Vec<String>,
    },
    ReviewDetails {
        title: String,
        lines: Vec<String>,
    },
    WorkerQueue {
        selected: usize,
        items: Vec<WorkerQueueItemSummary>,
    },
    WorkerDetails {
        title: String,
        lines: Vec<String>,
    },
    EntryDetails {
        entry_index: usize,
    },
    Timeline,
    Help,
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
    SessionQueue,
    Compact,
    Timeline,
    ReviewApproval,
    MissionControl,
    ReviewQueue,
    WorkerQueue,
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
            ("/help", "Open the operator help deck", Self::Help),
            (
                "/status",
                "Show the runtime and session control deck",
                Self::Status,
            ),
            (
                "/history",
                "Show the current transcript window summary",
                Self::History,
            ),
            (
                "Session queue",
                "Open the visible session/lineage inspector for this session scope",
                Self::SessionQueue,
            ),
            (
                "/compact",
                "Run manual compaction and checkpoint summary",
                Self::Compact,
            ),
            (
                "Timeline",
                "Open the transcript navigator overlay",
                Self::Timeline,
            ),
            (
                "Mission control",
                "Open the orchestration overview for the current session scope",
                Self::MissionControl,
            ),
            (
                "Review approval",
                "Reopen the latest approval request screen if one is pending",
                Self::ReviewApproval,
            ),
            (
                "Review queue",
                "Open the approval queue inspector for the current session",
                Self::ReviewQueue,
            ),
            (
                "Worker queue",
                "Open the visible delegate session/worker inspector",
                Self::WorkerQueue,
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
                "Show or hide the control deck",
                Self::ToggleSidebar,
            ),
            (
                "Cycle rail tab",
                "Move the control deck to the next tab",
                Self::CycleSidebarTab,
            ),
            (
                "Clear composer",
                "Clear the current draft",
                Self::ClearComposer,
            ),
            ("/exit", "Leave the session surface", Self::Exit),
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

#[derive(Clone, Debug, Default)]
struct ApprovalSurfaceSummary {
    title: String,
    subtitle: Option<String>,
    request_items: Vec<String>,
    rationale_lines: Vec<String>,
    choice_lines: Vec<String>,
    footer_lines: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct ApprovalQueueItemSummary {
    approval_request_id: String,
    status: String,
    tool_name: String,
    turn_id: String,
    requested_at: i64,
    reason: Option<String>,
    rule_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct WorkerQueueItemSummary {
    session_id: String,
    label: String,
    state: String,
    kind: String,
    parent_session_id: Option<String>,
    turn_count: usize,
    updated_at: i64,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct SessionQueueItemSummary {
    session_id: String,
    label: String,
    state: String,
    kind: String,
    parent_session_id: Option<String>,
    turn_count: usize,
    updated_at: i64,
    last_error: Option<String>,
}

impl ApprovalSurfaceSummary {
    fn from_screen_spec(screen: &TuiScreenSpec) -> Self {
        let mut request_items = Vec::new();
        let mut rationale_lines = Vec::new();

        for section in &screen.sections {
            match section {
                TuiSectionSpec::KeyValues { items, .. } => {
                    request_items.extend(items.iter().map(|item| match item {
                        TuiKeyValueSpec::Plain { key, value } => format!("{key}: {value}"),
                        TuiKeyValueSpec::Csv { key, values } => {
                            format!("{key}: {}", values.join(", "))
                        }
                    }));
                }
                TuiSectionSpec::Callout { lines, .. } | TuiSectionSpec::Narrative { lines, .. } => {
                    rationale_lines.extend(lines.clone());
                }
                TuiSectionSpec::ActionGroup { .. }
                | TuiSectionSpec::Checklist { .. }
                | TuiSectionSpec::Preformatted { .. } => {}
            }
        }

        let choice_lines = screen
            .choices
            .iter()
            .map(|choice| {
                if choice.recommended {
                    format!("{}: {} (recommended)", choice.key, choice.label)
                } else {
                    format!("{}: {}", choice.key, choice.label)
                }
            })
            .collect::<Vec<_>>();

        Self {
            title: screen
                .title
                .clone()
                .unwrap_or_else(|| "approval".to_owned()),
            subtitle: screen.subtitle.clone(),
            request_items,
            rationale_lines,
            choice_lines,
            footer_lines: screen.footer_lines.clone(),
        }
    }

    fn screen_spec(&self) -> TuiScreenSpec {
        let mut sections = Vec::new();
        if !self.rationale_lines.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("reason".to_owned()),
                lines: self.rationale_lines.clone(),
            });
        }
        if !self.request_items.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("request".to_owned()),
                lines: self.request_items.clone(),
            });
        }
        let choices = self
            .choice_lines
            .iter()
            .enumerate()
            .map(|(index, line)| TuiChoiceSpec {
                key: (index + 1).to_string(),
                label: line.clone(),
                detail_lines: Vec::new(),
                recommended: line.contains("(recommended)"),
            })
            .collect::<Vec<_>>();

        TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: self.subtitle.clone(),
            title: Some(self.title.clone()),
            progress_line: None,
            intro_lines: Vec::new(),
            sections,
            choices,
            footer_lines: self.footer_lines.clone(),
        }
    }
}

impl ApprovalQueueItemSummary {
    fn from_control_plane_summary(summary: &ChatControlPlaneApprovalSummary) -> Self {
        Self {
            approval_request_id: summary.approval_request_id.clone(),
            status: summary.status.clone(),
            tool_name: summary.tool_name.clone(),
            turn_id: summary.turn_id.clone(),
            requested_at: summary.requested_at,
            reason: summary.reason.clone(),
            rule_id: summary.rule_id.clone(),
            last_error: summary.last_error.clone(),
        }
    }

    fn list_line(&self) -> String {
        let reason = self.reason.as_deref().unwrap_or("-");
        format!(
            "{} status={} tool={} reason={}",
            self.approval_request_id, self.status, self.tool_name, reason
        )
    }

    fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("approval_request_id={}", self.approval_request_id),
            format!("status={}", self.status),
            format!("tool_name={}", self.tool_name),
            format!("turn_id={}", self.turn_id),
            format!("requested_at={}", self.requested_at),
        ];
        if let Some(reason) = self.reason.as_deref() {
            lines.push(format!("reason={reason}"));
        }
        if let Some(rule_id) = self.rule_id.as_deref() {
            lines.push(format!("rule_id={rule_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
}

impl WorkerQueueItemSummary {
    fn from_control_plane_summary(summary: &ChatControlPlaneSessionSummary) -> Self {
        Self {
            session_id: summary.session_id.clone(),
            label: summary.label.clone(),
            state: summary.state.clone(),
            kind: summary.kind.clone(),
            parent_session_id: summary.parent_session_id.clone(),
            turn_count: summary.turn_count,
            updated_at: summary.updated_at,
            last_error: summary.last_error.clone(),
        }
    }

    fn list_line(&self) -> String {
        format!(
            "{} state={} kind={} turns={}",
            self.label, self.state, self.kind, self.turn_count
        )
    }

    fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("session_id={}", self.session_id),
            format!("label={}", self.label),
            format!("state={}", self.state),
            format!("kind={}", self.kind),
            format!("turn_count={}", self.turn_count),
            format!("updated_at={}", self.updated_at),
        ];
        if let Some(parent_session_id) = self.parent_session_id.as_deref() {
            lines.push(format!("parent_session_id={parent_session_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
}

impl SessionQueueItemSummary {
    fn from_control_plane_summary(summary: &ChatControlPlaneSessionSummary) -> Self {
        Self {
            session_id: summary.session_id.clone(),
            label: summary.label.clone(),
            state: summary.state.clone(),
            kind: summary.kind.clone(),
            parent_session_id: summary.parent_session_id.clone(),
            turn_count: summary.turn_count,
            updated_at: summary.updated_at,
            last_error: summary.last_error.clone(),
        }
    }

    fn list_line(&self) -> String {
        format!(
            "{} state={} kind={} turns={}",
            self.label, self.state, self.kind, self.turn_count
        )
    }

    fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("session_id={}", self.session_id),
            format!("label={}", self.label),
            format!("state={}", self.state),
            format!("kind={}", self.kind),
            format!("turn_count={}", self.turn_count),
            format!("updated_at={}", self.updated_at),
        ];
        if let Some(parent_session_id) = self.parent_session_id.as_deref() {
            lines.push(format!("parent_session_id={parent_session_id}"));
        }
        if let Some(last_error) = self.last_error.as_deref() {
            lines.push(format!("last_error={last_error}"));
        }
        lines
    }
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
        let _ = self
            .term
            .write_str(terminal_surface_restore_sequence().as_str());
        let _ = self.term.flush();
    }
}

fn terminal_surface_restore_sequence() -> String {
    [
        BRACKETED_PASTE_DISABLE,
        CURSOR_KEYS_NORMAL,
        KEYPAD_NORMAL,
        ANSI_RESET,
        ALT_SCREEN_EXIT,
    ]
    .join("")
}

impl ChatSessionSurface {
    fn new(runtime: CliTurnRuntime, options: CliChatOptions) -> CliResult<Self> {
        let term = Term::stdout();
        let startup_summary =
            operator_surfaces::build_cli_chat_startup_summary(&runtime, &options)?;
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
                screen: operator_surfaces::build_cli_chat_startup_screen_spec(&startup_summary),
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
    fn handle_key(&self, key: Key) -> CliResult<SurfaceLoopAction> {
        match key {
            Key::CtrlC => Ok(SurfaceLoopAction::Exit),
            Key::Escape => {
                let mut state = self.lock_state();
                if matches!(state.overlay, Some(SurfaceOverlay::Welcome { .. })) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
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
                if let Some(SurfaceOverlay::SessionQueue { selected, .. }) = state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(SurfaceOverlay::ReviewQueue { selected, .. }) =
                    state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(SurfaceOverlay::WorkerQueue { selected, .. }) =
                    state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(palette) = state.command_palette.as_mut() {
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
                if let Some(SurfaceOverlay::SessionQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(SurfaceOverlay::ReviewQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(SurfaceOverlay::WorkerQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(palette) = state.command_palette.as_mut() {
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
                    if let Some(SurfaceOverlay::SessionQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        let detail_lines = self.build_session_detail_lines(item);
                        state.overlay = Some(SurfaceOverlay::SessionDetails {
                            title: format!("session {}", item.session_id),
                            lines: detail_lines,
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if let Some(SurfaceOverlay::ReviewQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        state.overlay = Some(SurfaceOverlay::ReviewDetails {
                            title: format!("approval {}", item.approval_request_id),
                            lines: item.detail_lines(),
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if let Some(SurfaceOverlay::WorkerQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        let detail_lines = self.build_worker_detail_lines(item);
                        state.overlay = Some(SurfaceOverlay::WorkerDetails {
                            title: format!("worker {}", item.session_id),
                            lines: detail_lines,
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
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
                if matches!(state.overlay, Some(SurfaceOverlay::Welcome { .. })) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                }
                if (character == ':' || character == '/')
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.command_palette = Some(CommandPaletteState::default());
                    state.focus = SurfaceFocus::CommandPalette;
                } else if character == '?'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.overlay = Some(SurfaceOverlay::Help);
                    state.focus = SurfaceFocus::Transcript;
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
                } else if character == 'M'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.try_build_mission_control_lines(&state, 10, 6, 6) {
                        Ok(lines) => {
                            state.overlay = Some(SurfaceOverlay::MissionControl { lines });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "mission",
                                "control plane",
                                error.as_str(),
                                vec![
                                    "Mission control needs a readable control-plane store before it can summarize related sessions."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'S'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_visible_sessions(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "sessions",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Related sessions and worker lanes will appear here when the control-plane store is available."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'r'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    if let Some(approval) = state.last_approval.as_ref() {
                        state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                            screen: approval.screen_spec(),
                        });
                        state.focus = SurfaceFocus::Transcript;
                    } else {
                        match self.load_review_queue_items(24) {
                            Ok(items) if !items.is_empty() => {
                                state.overlay =
                                    Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                                state.focus = SurfaceFocus::Transcript;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                let lines = render_control_plane_unavailable_lines_with_width(
                                    "review",
                                    "queue",
                                    error.as_str(),
                                    vec![
                                        "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                            .to_owned(),
                                    ],
                                    self.content_width(),
                                );
                                push_transcript_message(&mut state, lines);
                            }
                        }
                    }
                } else if character == 'R'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_review_queue_items(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "review",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'W'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_visible_worker_sessions(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::WorkerQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "workers",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Async delegate or worker sessions will appear here when the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
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
            CommandPaletteAction::SessionQueue => match self.load_visible_sessions(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "sessions".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("no visible sessions".to_owned()),
                                        lines: vec![
                                            "No visible sessions are currently rooted at this session scope."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Related sessions and worker lanes will appear here when they exist."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "sessions",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Related sessions and worker lanes will appear here when the control-plane store is available."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
            CommandPaletteAction::Compact => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_COMPACT_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Timeline => {
                state.overlay = Some(SurfaceOverlay::Timeline);
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::MissionControl => {
                match self.try_build_mission_control_lines(&state, 10, 6, 6) {
                    Ok(lines) => {
                        state.overlay = Some(SurfaceOverlay::MissionControl { lines });
                        state.focus = SurfaceFocus::Transcript;
                    }
                    Err(error) => {
                        push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "mission",
                                "control plane",
                                error.as_str(),
                                vec![
                                    "Mission control needs a readable control-plane store before it can summarize related sessions."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                    }
                }
            }
            CommandPaletteAction::ReviewApproval => {
                if let Some(approval) = state.last_approval.as_ref() {
                    state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                        screen: approval.screen_spec(),
                    });
                    state.focus = SurfaceFocus::Transcript;
                } else {
                    push_transcript_message(
                        &mut state,
                        render_cli_chat_message_spec_with_width(
                            &TuiMessageSpec {
                                role: "system".to_owned(),
                                caption: Some("review".to_owned()),
                                sections: vec![TuiSectionSpec::Callout {
                                    tone: TuiCalloutTone::Info,
                                    title: Some("no pending approval".to_owned()),
                                    lines: vec![
                                        "The latest turn does not have an approval screen to reopen."
                                            .to_owned(),
                                    ],
                                }],
                                footer_lines: vec![
                                    "Approvals appear automatically when governed actions pause the turn."
                                        .to_owned(),
                                ],
                            },
                            self.content_width(),
                        ),
                    );
                }
            }
            CommandPaletteAction::ReviewQueue => match self.load_review_queue_items(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "review".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("approval queue empty".to_owned()),
                                        lines: vec![
                                            "No approval requests are currently recorded for this session."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Governed actions will appear here after a turn pauses for approval."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "review",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
            CommandPaletteAction::WorkerQueue => match self.load_visible_worker_sessions(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "workers".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("no visible worker sessions".to_owned()),
                                        lines: vec![
                                            "No delegate child sessions are currently visible from this session scope."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Async delegate or worker sessions will appear here after they are spawned."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::WorkerQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "workers",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Async delegate or worker sessions will appear here when the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
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
        state.command_palette = None;
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

        let sessions_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_SESSIONS_COMMAND],
            "usage: /sessions",
        ))?;

        let mission_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_MISSION_COMMAND],
            "usage: /mission",
        ))?;

        let review_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_REVIEW_COMMAND],
            "usage: /review",
        ))?;

        let workers_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_WORKERS_COMMAND],
            "usage: /workers",
        ))?;

        let turn_checkpoint_repair_match = classify_chat_command_match_result(
            operator_surfaces::is_turn_checkpoint_repair_command(input),
        )?;

        if !matches!(sessions_match, ChatCommandMatchResult::NotMatched) {
            let lines = match sessions_match {
                ChatCommandMatchResult::Matched => match self.load_visible_sessions(24) {
                    Ok(items) => {
                        let session_queue_lines = items
                            .iter()
                            .take(12)
                            .map(SessionQueueItemSummary::list_line)
                            .collect::<Vec<_>>();
                        if !items.is_empty() {
                            let mut state = self.lock_state();
                            state.overlay =
                                Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                        }
                        render_cli_chat_message_spec_with_width(
                            &TuiMessageSpec {
                                role: "sessions".to_owned(),
                                caption: Some("visible lineage".to_owned()),
                                sections: vec![TuiSectionSpec::Narrative {
                                    title: Some("queue".to_owned()),
                                    lines: if session_queue_lines.is_empty() {
                                        vec![
                                            "No visible sessions are currently rooted at this session scope."
                                                .to_owned(),
                                        ]
                                    } else {
                                        session_queue_lines
                                    },
                                }],
                                footer_lines: vec![
                                    "Use S to open the session queue overlay or Enter on the queue to inspect one session."
                                        .to_owned(),
                                ],
                            },
                            width,
                        )
                    }
                    Err(error) => render_control_plane_unavailable_lines_with_width(
                        "sessions",
                        "visible lineage",
                        error.as_str(),
                        vec![
                            "Use /status or restore the control-plane store before inspecting related sessions."
                                .to_owned(),
                        ],
                        width,
                    ),
                },
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => {
                    render_cli_chat_command_usage_lines_with_width("usage: /sessions", width)
                }
            };

            let mut state = self.lock_state();
            push_transcript_message(&mut state, lines);
            return Ok(());
        }

        if !matches!(mission_match, ChatCommandMatchResult::NotMatched) {
            let lines = match mission_match {
                ChatCommandMatchResult::Matched => {
                    let mission_result = {
                        let state = self.lock_state();
                        self.try_build_mission_control_lines(&state, 10, 6, 6)
                    };
                    match mission_result {
                        Ok(mission_lines) => {
                            let mut state = self.lock_state();
                            state.overlay = Some(SurfaceOverlay::MissionControl {
                                lines: mission_lines.clone(),
                            });
                            state.focus = SurfaceFocus::Transcript;
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "mission".to_owned(),
                                    caption: Some("control plane".to_owned()),
                                    sections: vec![TuiSectionSpec::Narrative {
                                        title: Some("overview".to_owned()),
                                        lines: mission_lines,
                                    }],
                                    footer_lines: vec![
                                        "Use M to reopen mission control, S for sessions, W for workers, and R for approvals."
                                            .to_owned(),
                                    ],
                                },
                                width,
                            )
                        }
                        Err(error) => render_control_plane_unavailable_lines_with_width(
                            "mission",
                            "control plane",
                            error.as_str(),
                            vec![
                                "Mission control needs a readable control-plane store before it can summarize related sessions."
                                    .to_owned(),
                            ],
                            width,
                        ),
                    }
                }
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => {
                    render_cli_chat_command_usage_lines_with_width("usage: /mission", width)
                }
            };

            let mut state = self.lock_state();
            push_transcript_message(&mut state, lines);
            return Ok(());
        }

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
                        ChatCommandMatchResult::NotMatched => match review_match {
                            ChatCommandMatchResult::Matched => {
                                let maybe_lines = {
                                    let state = self.lock_state();
                                    state.last_approval.as_ref().map(|approval| {
                                        let review_queue_lines = match self.load_review_queue_items(6) {
                                            Ok(items) if items.is_empty() => vec!["approval queue: empty".to_owned()],
                                            Ok(items) => build_review_queue_lines_from_items(&items),
                                            Err(error) => vec![format!(
                                                "approval queue unavailable: {error}"
                                            )],
                                        };
                                        render_cli_chat_message_spec_with_width(
                                            &TuiMessageSpec {
                                                role: "review".to_owned(),
                                                caption: Some("latest approval".to_owned()),
                                                sections: vec![
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("queue".to_owned()),
                                                        lines: review_queue_lines,
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("title".to_owned()),
                                                        lines: vec![approval.title.clone()],
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("request".to_owned()),
                                                        lines: approval.request_items.clone(),
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("reason".to_owned()),
                                                        lines: approval.rationale_lines.clone(),
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("choices".to_owned()),
                                                        lines: approval.choice_lines.clone(),
                                                    },
                                                ],
                                                footer_lines: approval.footer_lines.clone(),
                                            },
                                            width,
                                        )
                                    })
                                };

                                if let Some(lines) = maybe_lines {
                                    let mut state = self.lock_state();
                                    if let Some(approval) = state.last_approval.as_ref() {
                                        state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                                            screen: approval.screen_spec(),
                                        });
                                    }
                                    lines
                                } else {
                                    match self.load_review_queue_items(6) {
                                        Ok(items) => {
                                            let review_queue_lines = if items.is_empty() {
                                                vec!["approval queue: empty".to_owned()]
                                            } else {
                                                build_review_queue_lines_from_items(&items)
                                            };
                                            render_cli_chat_message_spec_with_width(
                                                &TuiMessageSpec {
                                                    role: "review".to_owned(),
                                                    caption: Some("latest approval".to_owned()),
                                                    sections: vec![
                                                        TuiSectionSpec::Narrative {
                                                            title: Some("queue".to_owned()),
                                                            lines: review_queue_lines,
                                                        },
                                                        TuiSectionSpec::Callout {
                                                            tone: TuiCalloutTone::Info,
                                                            title: Some("no retained approval screen".to_owned()),
                                                            lines: vec![
                                                                "No approval/review item is currently retained in this session surface."
                                                                    .to_owned(),
                                                            ],
                                                        },
                                                    ],
                                                    footer_lines: vec![
                                                        "Governed actions automatically surface review screens when needed."
                                                            .to_owned(),
                                                    ],
                                                },
                                                width,
                                            )
                                        }
                                        Err(error) => render_control_plane_unavailable_lines_with_width(
                                            "review",
                                            "latest approval",
                                            error.as_str(),
                                            vec![
                                                "Governed actions automatically surface review screens when the control-plane store is readable."
                                                    .to_owned(),
                                            ],
                                            width,
                                        ),
                                    }
                                }
                            }
                            ChatCommandMatchResult::UsageError(usage) => {
                                render_cli_chat_command_usage_lines_with_width(&usage, width)
                            }
                            ChatCommandMatchResult::NotMatched => match workers_match {
                                ChatCommandMatchResult::Matched => match self.load_visible_worker_sessions(24) {
                                    Ok(items) => {
                                        let worker_queue_lines = items
                                            .iter()
                                            .take(12)
                                            .map(WorkerQueueItemSummary::list_line)
                                            .collect::<Vec<_>>();
                                        if !items.is_empty() {
                                            let mut state = self.lock_state();
                                            state.overlay = Some(SurfaceOverlay::WorkerQueue {
                                                selected: 0,
                                                items,
                                            });
                                        }
                                        render_cli_chat_message_spec_with_width(
                                            &TuiMessageSpec {
                                                role: "workers".to_owned(),
                                                caption: Some("visible delegates".to_owned()),
                                                sections: vec![TuiSectionSpec::Narrative {
                                                    title: Some("queue".to_owned()),
                                                    lines: if worker_queue_lines.is_empty() {
                                                        vec![
                                                            "No visible delegate child sessions are currently active in this session scope."
                                                                .to_owned(),
                                                        ]
                                                    } else {
                                                        worker_queue_lines
                                                    },
                                                }],
                                                footer_lines: vec![
                                                    "Use W to open the worker queue overlay or Enter on the queue to inspect one worker."
                                                        .to_owned(),
                                                ],
                                            },
                                            width,
                                        )
                                    }
                                    Err(error) => render_control_plane_unavailable_lines_with_width(
                                        "workers",
                                        "visible delegates",
                                        error.as_str(),
                                        vec![
                                            "Use /status or restore the control-plane store before inspecting worker sessions."
                                                .to_owned(),
                                        ],
                                        width,
                                    ),
                                },
                                ChatCommandMatchResult::UsageError(usage) => {
                                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                                }
                                ChatCommandMatchResult::NotMatched => {
                                    match turn_checkpoint_repair_match {
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
                                            render_cli_chat_command_usage_lines_with_width(
                                                &usage, width,
                                            )
                                        }
                                        ChatCommandMatchResult::NotMatched => {
                                            render_cli_chat_command_usage_lines_with_width(
                                                "usage: /help | /status | /history | /sessions | /mission | /review | /workers | /compact | /turn_checkpoint_repair | /exit",
                                                width,
                                            )
                                        }
                                    }
                                }
                            },
                        },
                    },
                },
            },
        };

        let mut state = self.lock_state();
        push_transcript_message(&mut state, lines);
        Ok(())
    }

    fn control_plane_store(&self) -> CliResult<ChatControlPlaneStore> {
        ChatControlPlaneStore::new(&self.runtime.memory_config)
    }

    fn try_build_mission_control_lines(
        &self,
        state: &SurfaceState,
        session_limit: usize,
        worker_limit: usize,
        review_limit: usize,
    ) -> CliResult<Vec<String>> {
        let visible_sessions =
            self.load_visible_sessions(session_limit.saturating_mul(2).max(8))?;
        let worker_items = visible_sessions
            .iter()
            .filter(|item| item.kind == CHAT_SESSION_KIND_DELEGATE_CHILD)
            .take(worker_limit)
            .cloned()
            .collect::<Vec<_>>();
        let approval_items = self.load_review_queue_items(review_limit)?;
        let maybe_snapshot = state.live.snapshot.as_ref();
        let phase = maybe_snapshot
            .map(|snapshot| snapshot.phase.as_str())
            .unwrap_or("idle");
        let provider_round = maybe_snapshot
            .and_then(|snapshot| snapshot.provider_round)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned());
        let tool_calls = maybe_snapshot
            .map(|snapshot| snapshot.tool_call_count)
            .unwrap_or(0);

        let visible_session_count = visible_sessions.len();
        let delegate_count = visible_sessions
            .iter()
            .filter(|item| item.kind == CHAT_SESSION_KIND_DELEGATE_CHILD)
            .count();
        let root_count = visible_session_count.saturating_sub(delegate_count);
        let failing_sessions = visible_sessions
            .iter()
            .filter(|item| item.state == "failed" || item.state == "timed_out")
            .count();

        let mut lines = vec![
            format!("scope: {}", self.runtime.session_id),
            format!("provider: {}", state.active_provider_label),
            format!("phase: {phase} · round={provider_round} · tools={tool_calls}"),
            format!(
                "lanes: sessions={} · roots={} · delegates={} · approvals={}",
                visible_session_count,
                root_count,
                delegate_count,
                approval_items.len()
            ),
        ];

        let session_state_mix =
            summarize_state_mix(visible_sessions.iter().map(|item| item.state.as_str()));
        if let Some(state_mix) = session_state_mix {
            lines.push(format!("session mix: {state_mix}"));
        }

        let worker_state_mix =
            summarize_state_mix(worker_items.iter().map(|item| item.state.as_str()));
        if let Some(worker_mix) = worker_state_mix {
            lines.push(format!("worker mix: {worker_mix}"));
        }

        if failing_sessions > 0 {
            lines.push(format!("attention: failing lanes={failing_sessions}"));
        }

        let recent_sessions = visible_sessions.iter().take(session_limit);
        let recent_session_lines = recent_sessions
            .map(SessionQueueItemSummary::list_line)
            .collect::<Vec<_>>();
        if !recent_session_lines.is_empty() {
            lines.push(String::new());
            lines.push("recent sessions".to_owned());
            lines.extend(recent_session_lines);
        }

        if !worker_items.is_empty() {
            lines.push(String::new());
            lines.push("recent workers".to_owned());
            lines.extend(worker_items.iter().map(SessionQueueItemSummary::list_line));
        }

        if !approval_items.is_empty() {
            lines.push(String::new());
            lines.push("review queue".to_owned());
            lines.extend(
                approval_items
                    .iter()
                    .take(review_limit)
                    .map(ApprovalQueueItemSummary::list_line),
            );
        }

        let maybe_approval = state.last_approval.as_ref();
        if let Some(approval) = maybe_approval {
            lines.push(String::new());
            lines.push(format!("latest approval: {}", approval.title));
            let maybe_subtitle = approval.subtitle.as_deref();
            if let Some(subtitle) = maybe_subtitle {
                lines.push(format!("mode: {subtitle}"));
            }
        }

        lines.push(String::new());
        lines.push("controls".to_owned());
        lines.push("S sessions · W workers · R approval queue".to_owned());
        lines.push("r latest approval · M mission control".to_owned());
        Ok(lines)
    }

    fn build_mission_control_lines(
        &self,
        state: &SurfaceState,
        session_limit: usize,
        worker_limit: usize,
        review_limit: usize,
    ) -> Vec<String> {
        match self.try_build_mission_control_lines(state, session_limit, worker_limit, review_limit)
        {
            Ok(lines) => lines,
            Err(error) => vec![format!("control_plane_unavailable={error}")],
        }
    }

    fn build_review_queue_lines(&self, limit: usize) -> Vec<String> {
        match self.load_review_queue_items(usize::MAX) {
            Ok(approval_items) => {
                if approval_items.is_empty() {
                    return vec!["approval queue: empty".to_owned()];
                }

                let total_count = approval_items.len();
                let mut lines = vec![format!("approval queue: {total_count}")];
                for item in approval_items.iter().take(limit) {
                    let list_line = item.list_line();
                    lines.push(list_line);

                    if let Some(reason) = item.reason.as_deref() {
                        lines.push(format!("  reason={reason}"));
                    }
                    if let Some(rule_id) = item.rule_id.as_deref() {
                        lines.push(format!("  rule_id={rule_id}"));
                    }
                    if let Some(last_error) = item.last_error.as_deref() {
                        lines.push(format!("  last_error={last_error}"));
                    }
                }
                lines
            }
            Err(error) => vec![format!("approval queue unavailable: {error}")],
        }
    }

    fn build_worker_queue_lines(&self, limit: usize) -> Vec<String> {
        match self.load_visible_worker_sessions(usize::MAX) {
            Ok(items) => {
                if items.is_empty() {
                    vec!["worker sessions: empty".to_owned()]
                } else {
                    let total_count = items.len();
                    let limited_items = items.into_iter().take(limit);
                    let mut lines = vec![format!("worker sessions: {total_count}")];
                    for item in limited_items {
                        let list_line = item.list_line();
                        lines.push(list_line);
                    }
                    lines
                }
            }
            Err(error) => {
                let error_line = format!("worker sessions unavailable: {error}");
                vec![error_line]
            }
        }
    }

    fn build_session_detail_lines(&self, item: &SessionQueueItemSummary) -> Vec<String> {
        let base_lines = item.detail_lines();
        let session_id = item.session_id.as_str();
        let include_delegate_lifecycle = false;

        self.build_session_detail_lines_with_runtime(
            session_id,
            base_lines,
            include_delegate_lifecycle,
        )
    }

    fn build_worker_detail_lines(&self, item: &WorkerQueueItemSummary) -> Vec<String> {
        let base_lines = item.detail_lines();
        let session_id = item.session_id.as_str();
        let include_delegate_lifecycle = true;

        self.build_session_detail_lines_with_runtime(
            session_id,
            base_lines,
            include_delegate_lifecycle,
        )
    }

    fn build_session_detail_lines_with_runtime(
        &self,
        session_id: &str,
        mut lines: Vec<String>,
        include_delegate_lifecycle: bool,
    ) -> Vec<String> {
        let store_result = self.control_plane_store();
        let store = match store_result {
            Ok(store) => store,
            Err(error) => {
                let detail_line = format!("detail_runtime_unavailable={error}");
                lines.push(detail_line);
                return lines;
            }
        };

        let details_result = store.session_details(session_id, include_delegate_lifecycle);
        let maybe_details = match details_result {
            Ok(details) => details,
            Err(error) => {
                let detail_line = format!("trajectory_unavailable={error}");
                lines.push(detail_line);
                return lines;
            }
        };

        let details = match maybe_details {
            Some(details) => details,
            None => {
                lines.push("trajectory_unavailable=session_not_found".to_owned());
                return lines;
            }
        };

        let maybe_lineage_root = details.lineage_root_session_id.as_deref();
        let lineage_root = maybe_lineage_root.unwrap_or("-");
        let turn_count = details.trajectory_turn_count;
        let event_count = details.event_count;
        let approval_count = details.approval_count;

        lines.push(String::new());
        lines.push(format!("lineage_root_session_id={lineage_root}"));
        lines.push(format!("lineage_depth={}", details.lineage_depth));
        lines.push(format!("trajectory_turn_count={turn_count}"));
        lines.push(format!("trajectory_event_count={event_count}"));
        lines.push(format!("approval_request_count={approval_count}"));

        let maybe_terminal_status = details.terminal_status.as_deref();
        let maybe_terminal_recorded_at = details.terminal_recorded_at;
        if let Some(terminal_status) = maybe_terminal_status {
            lines.push(format!("terminal_status={terminal_status}"));
        }
        if let Some(terminal_recorded_at) = maybe_terminal_recorded_at {
            lines.push(format!("terminal_recorded_at={terminal_recorded_at}"));
        }

        let maybe_last_turn_role = details.last_turn_role.as_deref();
        let maybe_last_turn_ts = details.last_turn_ts;
        let maybe_last_turn_excerpt = details.last_turn_excerpt.as_deref();
        if let Some(last_turn_role) = maybe_last_turn_role {
            lines.push(format!("last_turn_role={last_turn_role}"));
        }
        if let Some(last_turn_ts) = maybe_last_turn_ts {
            lines.push(format!("last_turn_ts={last_turn_ts}"));
        }
        if let Some(last_turn_excerpt) = maybe_last_turn_excerpt {
            lines.push(format!("last_turn_excerpt={last_turn_excerpt}"));
        }

        if !details.recent_events.is_empty() {
            lines.push(String::new());
            lines.push("recent_events".to_owned());
            lines.extend(details.recent_events);
        }

        let approval_items_result = store.approval_queue(session_id, 1);
        let approval_items = approval_items_result.unwrap_or_default();
        let maybe_latest_approval = approval_items.first();
        if let Some(latest_approval) = maybe_latest_approval {
            let approval_id = latest_approval.approval_request_id.as_str();
            let approval_status = latest_approval.status.as_str();
            let approval_tool = latest_approval.tool_name.as_str();
            lines.push(String::new());
            lines.push(format!("latest_approval_id={approval_id}"));
            lines.push(format!("latest_approval_status={approval_status}"));
            lines.push(format!("latest_approval_tool={approval_tool}"));
        }

        if !details.delegate_events.is_empty() {
            lines.push(String::new());
            lines.push("delegate_lifecycle".to_owned());
            lines.extend(details.delegate_events);
        }

        lines
    }

    fn load_review_queue_items(&self, limit: usize) -> CliResult<Vec<ApprovalQueueItemSummary>> {
        let store = self.control_plane_store()?;

        let approvals = store.approval_queue(&self.runtime.session_id, limit)?;

        let mut items = Vec::new();
        for approval in approvals {
            let item = ApprovalQueueItemSummary::from_control_plane_summary(&approval);
            items.push(item);
        }
        Ok(items)
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
                    participant_id: self.runtime.session_address.participant_id.clone(),
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
                state.last_approval = Some(ApprovalSurfaceSummary::from_screen_spec(&screen));
                state.overlay = Some(SurfaceOverlay::ApprovalPrompt { screen });
            } else {
                state.last_approval = None;
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
        let render_data = SurfaceRenderData {
            header_lines,
            header_status_line: self
                .build_header_status_line(&state, total_width.saturating_sub(4)),
            transcript_lines: self.build_transcript_lines(&state, content_width, transcript_height),
            sidebar_visible,
            sidebar_tab: state.sidebar_tab,
            sidebar_lines: self.build_sidebar_lines(
                &state,
                SIDEBAR_WIDTH.saturating_sub(2),
                transcript_height,
            ),
            composer_lines: self.build_composer_lines(&state, total_width.saturating_sub(6)),
            status_line: self.build_status_line(&state, total_width.saturating_sub(4)),
        };
        let output =
            render_surface_to_string(&state, &render_data, Rect::new(0, 0, width_u16, height_u16));

        self.term
            .write_str(format!("{CLEAR_AND_HOME}{output}").as_str())
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
            format!("control deck · {}", state.sidebar_tab.title()),
            format!("session {}", startup_summary.session_id),
        ];
        lines.push(format!("focus: {}", state.focus.label()));
        let tab_label = format!(
            "tabs: {} | {} | {} | {} | {} | {} | {}",
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
            if state.sidebar_tab == SidebarTab::Mission {
                "[mission]"
            } else {
                "mission"
            },
            if state.sidebar_tab == SidebarTab::Workers {
                "[workers]"
            } else {
                "workers"
            },
            if state.sidebar_tab == SidebarTab::Review {
                "[review]"
            } else {
                "review"
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
            SidebarTab::Mission => {
                lines.extend(self.build_mission_control_lines(state, 4, 3, 3));
            }
            SidebarTab::Workers => {
                lines.extend(self.build_worker_queue_lines(6));
            }
            SidebarTab::Review => {
                let queue_lines = self.build_review_queue_lines(4);
                lines.extend(queue_lines);
                if let Some(approval) = state.last_approval.as_ref() {
                    lines.push(String::new());
                    lines.push(format!("approval: {}", approval.title));
                    if let Some(subtitle) = approval.subtitle.as_deref() {
                        lines.push(format!("mode: {subtitle}"));
                    }
                    if !approval.request_items.is_empty() {
                        lines.push("request".to_owned());
                        lines.extend(approval.request_items.iter().take(4).cloned());
                    }
                    if !approval.rationale_lines.is_empty() {
                        lines.push("reason".to_owned());
                        lines.extend(approval.rationale_lines.iter().take(4).cloned());
                    }
                    if !approval.choice_lines.is_empty() {
                        lines.push("choices".to_owned());
                        lines.extend(approval.choice_lines.iter().take(4).cloned());
                    }
                } else if lines.is_empty() {
                    lines.push("no pending approval".to_owned());
                    lines.push("Governed actions surface here.".to_owned());
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
                lines.push("M open mission control".to_owned());
                lines.push("r reopen latest approval".to_owned());
                lines.push("S open session queue".to_owned());
                lines.push("W open worker queue".to_owned());
                lines.push("R open approval queue".to_owned());
                lines.push("← / → / Home / End composer cursor".to_owned());
                lines.push("↑ / ↓ composer multiline move".to_owned());
                lines.push("?: help overlay".to_owned());
                lines.push(": or / command menu".to_owned());
                lines.push(
                    "/help /status /history /sessions /mission /review /workers /compact"
                        .to_owned(),
                );
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
            "╰─ command menu active · type filter · ↑↓ choose · Enter run · Esc close"
        } else if state.composer.starts_with('/') {
            "╰─ slash mode · Enter send command · : or / opens the command menu"
        } else if should_continue_multiline(&state.composer) {
            "╰─ multiline compose · trailing \\ inserts newline on Enter"
        } else {
            "╰─ Enter send · ? help · : or / command menu"
        };
        vec![prompt_line, body_line, second_line, hint.to_owned()]
    }

    fn build_status_line(&self, state: &SurfaceState, width: usize) -> String {
        let mut status = format!(
            "{} · mode=chat · focus={} · deck={} · entries={} · scroll={} · sticky={} · overlay={}",
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

    fn build_header_status_line(&self, state: &SurfaceState, width: usize) -> String {
        let session_id = state
            .startup_summary
            .as_ref()
            .map(|summary| summary.session_id.as_str())
            .unwrap_or(self.runtime.session_id.as_str());
        let acp = if self.runtime.config.acp.enabled {
            "acp:on"
        } else {
            "acp:off"
        };
        clipped_display_line(
            format!(
                "session={session_id} · provider={} · {} · focus={} · overlay={}",
                state.active_provider_label,
                acp,
                state.focus.label(),
                current_overlay_label(state)
            )
            .as_str(),
            width,
        )
    }

    #[allow(dead_code)]
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
            "╭─ command menu".to_owned()
        } else {
            format!("╭─ command menu · query={}", palette.query)
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    fn build_prompt_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        match state.overlay.as_ref()? {
            SurfaceOverlay::Welcome { screen } => {
                let overlay_width = min(total_width.saturating_sub(8), 92).max(40);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(20)) / 2;
                let lines = render_tui_screen_spec(screen, overlay_width.saturating_sub(4), false);
                let mut rendered = vec![format!("\x1b[{};{}H╭─ welcome", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(16).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Type to begin · ? help · : command menu · Esc close",
                    y + 18,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::MissionControl { lines } => {
                let overlay_width = min(total_width.saturating_sub(8), 92).max(40);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(20)) / 2;
                let mut rendered = vec![format!("\x1b[{};{}H╭─ mission control", y + 1, x + 1)];
                for (offset, line) in lines.iter().take(16).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Esc close · S sessions · W workers · R approvals",
                    y + 18,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::Help => {
                let overlay_width = min(total_width.saturating_sub(10), 88).max(36);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(16)) / 2;
                let lines = operator_surfaces::render_cli_chat_help_lines_with_width(
                    overlay_width.saturating_sub(4),
                );
                let mut rendered = vec![format!("\x1b[{};{}H╭─ help", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(12).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Esc close · : command menu · /help send command",
                    y + 14,
                    x + 1
                ));
                Some(rendered.join(""))
            }
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
            SurfaceOverlay::ReviewQueue { .. }
            | SurfaceOverlay::ReviewDetails { .. }
            | SurfaceOverlay::SessionQueue { .. }
            | SurfaceOverlay::SessionDetails { .. }
            | SurfaceOverlay::WorkerQueue { .. }
            | SurfaceOverlay::WorkerDetails { .. }
            | SurfaceOverlay::EntryDetails { .. }
            | SurfaceOverlay::Timeline => None,
        }
    }

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
            Some(session_surface_subtitle(state)),
        );
        let header_height = header_lines.len();
        let reserved_height = header_height + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;

        total_height.saturating_sub(reserved_height).max(5)
    }
}

fn push_transcript_message(state: &mut SurfaceState, lines: Vec<String>) {
    state.transcript.push(SurfaceEntry { lines });
    state.selected_entry = Some(state.transcript.len().saturating_sub(1));
    state.sticky_bottom = true;
    state.focus = SurfaceFocus::Transcript;
}

fn control_plane_unavailable_title(error: &str) -> &'static str {
    if error.contains("memory-sqlite support") {
        "feature unavailable"
    } else {
        "control plane unavailable"
    }
}

fn render_control_plane_unavailable_lines_with_width(
    role: &str,
    caption: &str,
    error: &str,
    footer_lines: Vec<String>,
    width: usize,
) -> Vec<String> {
    let detail = format!("{role} unavailable: {error}");
    render_cli_chat_message_spec_with_width(
        &TuiMessageSpec {
            role: role.to_owned(),
            caption: Some(caption.to_owned()),
            sections: vec![TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some(control_plane_unavailable_title(error).to_owned()),
                lines: vec![detail],
            }],
            footer_lines,
        },
        width,
    )
}

fn build_review_queue_lines_from_items(approval_items: &[ApprovalQueueItemSummary]) -> Vec<String> {
    if approval_items.is_empty() {
        return vec!["approval queue: empty".to_owned()];
    }

    let total_count = approval_items.len();
    let mut lines = vec![format!("approval queue: {total_count}")];

    for item in approval_items {
        let list_line = item.list_line();
        lines.push(list_line);

        let maybe_reason = item.reason.as_deref();
        if let Some(reason) = maybe_reason {
            lines.push(format!("  reason={reason}"));
        }

        let maybe_rule_id = item.rule_id.as_deref();
        if let Some(rule_id) = maybe_rule_id {
            lines.push(format!("  rule_id={rule_id}"));
        }

        let maybe_last_error = item.last_error.as_deref();
        if let Some(last_error) = maybe_last_error {
            lines.push(format!("  last_error={last_error}"));
        }
    }

    lines
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

struct SurfaceRenderData {
    header_lines: Vec<String>,
    header_status_line: String,
    transcript_lines: Vec<String>,
    sidebar_visible: bool,
    sidebar_tab: SidebarTab,
    sidebar_lines: Vec<String>,
    composer_lines: Vec<String>,
    status_line: String,
}

fn render_surface_to_string(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
) -> String {
    if area.width == 0 || area.height == 0 {
        return String::new();
    }

    let mut buffer = Buffer::empty(area);
    let composer_height = u16::try_from(render_data.composer_lines.len().max(2))
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let header_height = u16::try_from(render_data.header_lines.len().max(1))
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let footer_height = 3;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height.min(area.height.saturating_sub(footer_height + 4))),
            Constraint::Min(6),
            Constraint::Length(composer_height.min(area.height.saturating_sub(footer_height + 2))),
            Constraint::Length(footer_height),
        ])
        .split(area);
    let header_area = rect_or(layout.as_ref(), 0, area);
    let body_area = rect_or(layout.as_ref(), 1, area);
    let composer_area = rect_or(layout.as_ref(), 2, area);
    let footer_area = rect_or(layout.as_ref(), 3, area);

    render_surface_header(render_data, header_area, &mut buffer);
    render_surface_body(state, render_data, body_area, &mut buffer);
    render_surface_composer(render_data, composer_area, &mut buffer);
    render_surface_footer(render_data, footer_area, &mut buffer);
    render_surface_overlays(state, body_area, &mut buffer);

    render_buffer_to_string(&buffer)
}

fn render_surface_header(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" loongclaw / chat ");
    let inner = block.inner(area);
    block.render(area, buffer);
    if inner.height == 0 {
        return;
    }

    let status_height = 1;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(status_height)])
        .split(inner);
    let brand_area = rect_or(layout.as_ref(), 0, inner);
    let status_area = rect_or(layout.as_ref(), 1, inner);

    Paragraph::new(text_from_lines(&render_data.header_lines))
        .wrap(Wrap { trim: false })
        .render(brand_area, buffer);
    Paragraph::new(render_data.header_status_line.clone()).render(status_area, buffer);
}

fn render_surface_body(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
    buffer: &mut Buffer,
) {
    if render_data.sidebar_visible {
        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(40),
                Constraint::Length(SIDEBAR_WIDTH as u16),
            ])
            .split(area);
        let transcript_area = rect_or(layout.as_ref(), 0, area);
        let sidebar_area = rect_or(layout.as_ref(), 1, area);
        render_transcript_panel(state, render_data, transcript_area, buffer);
        render_sidebar_panel(render_data, sidebar_area, buffer);
    } else {
        render_transcript_panel(state, render_data, area, buffer);
    }
}

fn render_transcript_panel(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
    buffer: &mut Buffer,
) {
    let title = if state.pending_turn {
        " transcript · live turn "
    } else {
        " transcript "
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(text_from_lines(&render_data.transcript_lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_sidebar_panel(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" control deck ");
    let inner = block.inner(area);
    block.render(area, buffer);
    if inner.height == 0 {
        return;
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);
    let tabs_area = rect_or(layout.as_ref(), 0, inner);
    let body_area = rect_or(layout.as_ref(), 1, inner);

    let tab_titles = [
        SidebarTab::Session,
        SidebarTab::Runtime,
        SidebarTab::Tools,
        SidebarTab::Mission,
        SidebarTab::Workers,
        SidebarTab::Review,
        SidebarTab::Help,
    ]
    .into_iter()
    .map(|tab| {
        let label = tab.title();
        if tab == render_data.sidebar_tab {
            Line::from(format!("[{label}]"))
        } else {
            Line::from(label)
        }
    })
    .collect::<Vec<_>>();
    Tabs::new(tab_titles).render(tabs_area, buffer);
    Paragraph::new(text_from_lines(&render_data.sidebar_lines))
        .wrap(Wrap { trim: false })
        .render(body_area, buffer);
}

fn render_surface_composer(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default().borders(Borders::ALL).title(" compose ");
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(text_from_lines(&render_data.composer_lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_surface_footer(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default().borders(Borders::TOP).title(" controls ");
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(render_data.status_line.clone()).render(inner, buffer);
}

fn render_surface_overlays(state: &SurfaceState, overlay_area: Rect, buffer: &mut Buffer) {
    if let Some(palette) = state.command_palette.as_ref() {
        let items = filtered_command_palette_items(&palette.query)
            .into_iter()
            .enumerate()
            .map(|(index, (label, detail, _))| {
                let marker = if index == palette.selected { ">" } else { " " };
                ListItem::new(format!("{marker} {label} — {detail}"))
            })
            .collect::<Vec<_>>();
        let title = if palette.query.is_empty() {
            " command menu ".to_owned()
        } else {
            format!(" command menu · {} ", palette.query)
        };
        render_overlay_list(
            overlay_area,
            68,
            14,
            title.as_str(),
            if items.is_empty() {
                vec![ListItem::new("no commands match the current query")]
            } else {
                items
            },
            buffer,
        );
    }

    match state.overlay.as_ref() {
        Some(SurfaceOverlay::Welcome { screen }) => {
            render_overlay_paragraph(
                overlay_area,
                92,
                20,
                " welcome ",
                &render_tui_screen_spec(screen, 84, false),
                buffer,
            );
        }
        Some(SurfaceOverlay::MissionControl { lines }) => {
            render_overlay_paragraph(overlay_area, 92, 20, " mission control ", lines, buffer);
        }
        Some(SurfaceOverlay::SessionQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " session queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::SessionDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::ReviewQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " review queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::ReviewDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::WorkerQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " worker queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::WorkerDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::EntryDetails { entry_index }) => {
            if let Some(entry) = state.transcript.get(*entry_index) {
                render_overlay_paragraph(
                    overlay_area,
                    88,
                    18,
                    format!(" entry details · #{} ", entry_index + 1).as_str(),
                    &entry.lines,
                    buffer,
                );
            }
        }
        Some(SurfaceOverlay::Timeline) => {
            let selected = state
                .selected_entry
                .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
            let items = state
                .transcript
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let prefix = if index == selected { ">" } else { " " };
                    let title = entry
                        .lines
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "(empty entry)".to_owned());
                    ListItem::new(format!("{prefix} {:>3}. {}", index + 1, title))
                })
                .collect::<Vec<_>>();
            render_overlay_list(overlay_area, 72, 18, " timeline ", items, buffer);
        }
        Some(SurfaceOverlay::Help) => {
            render_overlay_paragraph(
                overlay_area,
                88,
                16,
                " help ",
                &operator_surfaces::render_cli_chat_help_lines_with_width(82),
                buffer,
            );
        }
        Some(SurfaceOverlay::ConfirmExit) => {
            render_overlay_paragraph(
                overlay_area,
                60,
                7,
                " confirm exit ",
                &[
                    "Press Enter to leave the session surface, or Esc to continue.".to_owned(),
                    String::new(),
                    "Enter confirm · Esc cancel".to_owned(),
                ],
                buffer,
            );
        }
        Some(SurfaceOverlay::InputPrompt {
            kind,
            value,
            cursor,
        }) => {
            let title = match kind {
                OverlayInputKind::RenameSession => " rename session ",
                OverlayInputKind::ExportTranscript => " export transcript ",
            };
            let hint = match kind {
                OverlayInputKind::RenameSession => {
                    "Set a local session title for this fullscreen surface."
                }
                OverlayInputKind::ExportTranscript => {
                    "Choose a file path to write the current transcript."
                }
            };
            let lines = vec![
                hint.to_owned(),
                String::new(),
                composer_text_with_cursor(value, *cursor),
                String::new(),
                "Enter save · Esc cancel".to_owned(),
            ];
            render_overlay_paragraph(overlay_area, 72, 9, title, &lines, buffer);
        }
        Some(SurfaceOverlay::ApprovalPrompt { screen }) => {
            render_overlay_paragraph(
                overlay_area,
                88,
                16,
                " approval required ",
                &render_tui_screen_spec(screen, 82, false),
                buffer,
            );
        }
        None => {}
    }
}

fn render_overlay_paragraph(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    title: &str,
    lines: &[String],
    buffer: &mut Buffer,
) {
    let overlay_area = centered_rect(area, desired_width, desired_height);
    Clear.render(overlay_area, buffer);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(overlay_area);
    block.render(overlay_area, buffer);
    Paragraph::new(text_from_lines(lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_overlay_list(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    title: &str,
    items: Vec<ListItem<'static>>,
    buffer: &mut Buffer,
) {
    let overlay_area = centered_rect(area, desired_width, desired_height);
    Clear.render(overlay_area, buffer);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(overlay_area);
    block.render(overlay_area, buffer);
    List::new(items).render(inner, buffer);
}

fn centered_rect(area: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.min(area.width.saturating_sub(2)).max(10);
    let height = desired_height.min(area.height.saturating_sub(2)).max(5);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    let vertical_area = rect_or(vertical.as_ref(), 1, area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical_area);
    rect_or(horizontal.as_ref(), 1, vertical_area)
}

fn text_from_lines(lines: &[String]) -> Text<'static> {
    Text::from(lines.iter().cloned().map(Line::from).collect::<Vec<_>>())
}

fn rect_or(layout: &[Rect], index: usize, fallback: Rect) -> Rect {
    layout.get(index).copied().unwrap_or(fallback)
}

fn render_buffer_to_string(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut rendered_lines = Vec::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        rendered_lines.push(line.trim_end_matches(' ').to_owned());
    }
    rendered_lines.join("\n")
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
        format!("session: {}", startup_summary.session_id),
        format!("focus: {}", snapshot_state.focus.label()),
        format!("sticky: {}", snapshot_state.sticky_bottom),
        format!("phase: {}", snapshot_state.live.last_phase_label),
    ];
    if let Some(preview) = snapshot_state.live.last_assistant_preview.as_deref() {
        sidebar_lines.push(String::new());
        sidebar_lines.push("last reply".to_owned());
        sidebar_lines.extend(
            crate::presentation::render_wrapped_display_line(
                preview,
                SIDEBAR_WIDTH.saturating_sub(4),
            )
            .into_iter()
            .take(8),
        );
    }
    let draft_lines = composer_display_lines(
        &composer_text_with_cursor(&snapshot_state.composer, snapshot_state.composer_cursor),
        total_width.saturating_sub(6),
        2,
    );
    let composer_lines = vec![
        format!("draft · focus={}", snapshot_state.focus.label()),
        draft_lines.first().cloned().unwrap_or_default(),
        if draft_lines.len() > 1 {
            draft_lines.get(1).cloned().unwrap_or_default()
        } else if let Some(hint) = slash_command_hint(&snapshot_state.composer) {
            hint
        } else {
            "turn running…".to_owned()
        },
        "Enter send · ? help · : or / command menu".to_owned(),
    ];
    let mut status_text = format!(
        "?: help · : command menu · M mission · Esc clear · PgUp/PgDn transcript · Tab focus · focus={} · deck={} · sticky={}",
        snapshot_state.focus.label(),
        snapshot_state.sidebar_tab.title(),
        snapshot_state.sticky_bottom
    );
    if snapshot_state.pending_turn {
        status_text.push_str(" · turn running");
    }
    let render_data = SurfaceRenderData {
        header_lines,
        header_status_line: clipped_display_line(
            format!(
                "session={} · provider={} · phase={} · focus={} · overlay={}",
                startup_summary.session_id,
                snapshot_state.active_provider_label,
                snapshot_state.live.last_phase_label,
                snapshot_state.focus.label(),
                current_overlay_label(&snapshot_state)
            )
            .as_str(),
            total_width.saturating_sub(4),
        ),
        transcript_lines,
        sidebar_visible,
        sidebar_tab: snapshot_state.sidebar_tab,
        sidebar_lines,
        composer_lines,
        status_line: clipped_display_line(status_text.as_str(), total_width.saturating_sub(4)),
    };
    let output = render_surface_to_string(
        &snapshot_state,
        &render_data,
        Rect::new(0, 0, width_u16, height_u16),
    );
    term.write_str(format!("{CLEAR_AND_HOME}{output}").as_str())
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

fn summarize_state_mix<'a>(states: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut ready = 0usize;
    let mut running = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut timed_out = 0usize;
    let mut other = 0usize;
    let mut seen_any = false;

    for state in states {
        seen_any = true;
        match state {
            "ready" => ready += 1,
            "running" => running += 1,
            "completed" => completed += 1,
            "failed" => failed += 1,
            "timed_out" => timed_out += 1,
            _ => other += 1,
        }
    }

    if !seen_any {
        return None;
    }

    let mut parts = Vec::new();
    if ready > 0 {
        parts.push(format!("ready={ready}"));
    }
    if running > 0 {
        parts.push(format!("running={running}"));
    }
    if completed > 0 {
        parts.push(format!("completed={completed}"));
    }
    if failed > 0 {
        parts.push(format!("failed={failed}"));
    }
    if timed_out > 0 {
        parts.push(format!("timed_out={timed_out}"));
    }
    if other > 0 {
        parts.push(format!("other={other}"));
    }

    Some(parts.join(" · "))
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
        .unwrap_or("operator cockpit")
}

fn default_export_path(session_id: &str) -> String {
    let sanitized_session_id = sanitize_session_id_for_export(session_id);
    let file_name = format!("loong-{sanitized_session_id}-transcript.txt");
    let exports_dir = loong_exports_dir();
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

fn loong_exports_dir() -> PathBuf {
    let loong_home = crate::config::default_loong_home();
    loong_home.join("exports")
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
        Some(SurfaceOverlay::Welcome { .. }) => "welcome",
        Some(SurfaceOverlay::MissionControl { .. }) => "mission",
        Some(SurfaceOverlay::SessionQueue { .. }) => "session-queue",
        Some(SurfaceOverlay::SessionDetails { .. }) => "session-detail",
        Some(SurfaceOverlay::ReviewQueue { .. }) => "review-queue",
        Some(SurfaceOverlay::ReviewDetails { .. }) => "review-detail",
        Some(SurfaceOverlay::WorkerQueue { .. }) => "worker-queue",
        Some(SurfaceOverlay::WorkerDetails { .. }) => "worker-detail",
        Some(SurfaceOverlay::EntryDetails { .. }) => "entry",
        Some(SurfaceOverlay::Timeline) => "timeline",
        Some(SurfaceOverlay::Help) => "help",
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
        CLI_CHAT_SESSIONS_COMMAND,
        CLI_CHAT_MISSION_COMMAND,
        CLI_CHAT_REVIEW_COMMAND,
        CLI_CHAT_WORKERS_COMMAND,
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

    fn sample_surface_state() -> SurfaceState {
        SurfaceState {
            startup_summary: Some(fallback_startup_summary("default")),
            active_provider_label: "OpenAI / gpt-5.4".to_owned(),
            session_title_override: None,
            last_approval: None,
            transcript: vec![SurfaceEntry {
                lines: vec![
                    "you · prompt".to_owned(),
                    "Summarize the repository.".to_owned(),
                ],
            }],
            composer: "hi".to_owned(),
            composer_cursor: 2,
            history: Vec::new(),
            history_index: None,
            scroll_offset: 0,
            sticky_bottom: true,
            selected_entry: Some(0),
            focus: SurfaceFocus::Composer,
            sidebar_visible: true,
            sidebar_tab: SidebarTab::Runtime,
            command_palette: None,
            overlay: None,
            live: LiveSurfaceModel::default(),
            footer_notice: "?: help · : command menu".to_owned(),
            pending_turn: false,
        }
    }

    fn sample_render_data() -> SurfaceRenderData {
        SurfaceRenderData {
            header_lines: vec![
                "LOONGCLAW  v0.1.0-alpha.3".to_owned(),
                "interactive chat".to_owned(),
            ],
            header_status_line:
                "session=default · provider=OpenAI / gpt-5.4 · acp:off · focus=composer".to_owned(),
            transcript_lines: vec![
                "▶ you · prompt".to_owned(),
                "Summarize the repository.".to_owned(),
                String::new(),
                "assistant · reply".to_owned(),
                "Repository mapped.".to_owned(),
            ],
            sidebar_visible: true,
            sidebar_tab: SidebarTab::Runtime,
            sidebar_lines: vec![
                "session: default".to_owned(),
                "config: ~/.loong/config.toml".to_owned(),
                "memory: ~/.loong/memory.sqlite3".to_owned(),
            ],
            composer_lines: vec![
                "draft · focus=composer".to_owned(),
                "hi▏".to_owned(),
                String::new(),
                "Enter send · ? help · : or / command menu".to_owned(),
            ],
            status_line: "?: help · : command menu · Esc clear · PgUp/PgDn transcript · Tab focus"
                .to_owned(),
        }
    }

    #[test]
    fn sidebar_tab_cycles_forward_and_backward() {
        assert_eq!(SidebarTab::Session.next(), SidebarTab::Runtime);
        assert_eq!(SidebarTab::Runtime.next(), SidebarTab::Tools);
        assert_eq!(SidebarTab::Tools.next(), SidebarTab::Mission);
        assert_eq!(SidebarTab::Mission.next(), SidebarTab::Workers);
        assert_eq!(SidebarTab::Workers.next(), SidebarTab::Review);
        assert_eq!(SidebarTab::Review.next(), SidebarTab::Help);
        assert_eq!(SidebarTab::Help.next(), SidebarTab::Session);
        assert_eq!(SidebarTab::Session.previous(), SidebarTab::Help);
        assert_eq!(SidebarTab::Workers.previous(), SidebarTab::Mission);
        assert_eq!(SidebarTab::Review.previous(), SidebarTab::Workers);
        assert_eq!(SidebarTab::Help.previous(), SidebarTab::Review);
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
        assert_eq!(items[0].0, "/help");
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
        let mission_hint = slash_command_hint("/mi").expect("mission hint");
        let sessions_hint = slash_command_hint("/se").expect("sessions hint");

        assert!(hint.contains("/history"));
        assert!(mission_hint.contains("/mission"));
        assert!(sessions_hint.contains("/sessions"));
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

        assert!(labels.contains(&"Mission control"));
        assert!(labels.contains(&"Jump to latest"));
        assert!(labels.contains(&"Toggle sticky scroll"));
        assert!(labels.contains(&"Timeline"));
    }

    #[test]
    fn filtered_command_palette_items_respects_query() {
        let filtered = filtered_command_palette_items("time");
        assert!(filtered.iter().any(|item| item.0 == "Timeline"));
        assert!(!filtered.iter().any(|item| item.0 == "/compact"));
    }

    #[test]
    fn current_overlay_label_reports_overlay_kind() {
        let mut state = SurfaceState {
            startup_summary: None,
            active_provider_label: "provider / model".to_owned(),
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
            overlay: None,
            live: LiveSurfaceModel::default(),
            footer_notice: String::new(),
            pending_turn: false,
        };
        assert_eq!(current_overlay_label(&state), "none");
        state.overlay = Some(SurfaceOverlay::Welcome {
            screen: TuiScreenSpec {
                header_style: TuiHeaderStyle::Compact,
                subtitle: Some("interactive chat".to_owned()),
                title: Some("operator cockpit ready".to_owned()),
                progress_line: None,
                intro_lines: Vec::new(),
                sections: Vec::new(),
                choices: Vec::new(),
                footer_lines: Vec::new(),
            },
        });
        assert_eq!(current_overlay_label(&state), "welcome");
        state.overlay = Some(SurfaceOverlay::MissionControl {
            lines: vec!["scope: default".to_owned()],
        });
        assert_eq!(current_overlay_label(&state), "mission");
        state.overlay = Some(SurfaceOverlay::Timeline);
        assert_eq!(current_overlay_label(&state), "timeline");
        state.overlay = Some(SurfaceOverlay::Help);
        assert_eq!(current_overlay_label(&state), "help");
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
    fn default_export_path_uses_loong_exports_directory() {
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
        assert_eq!(file_name, "loong-session__bad-transcript.txt");
    }

    #[test]
    fn terminal_surface_restore_sequence_resets_terminal_modes_before_exit() {
        let sequence = terminal_surface_restore_sequence();

        assert!(
            sequence.contains(BRACKETED_PASTE_DISABLE),
            "restore sequence should disable bracketed paste: {sequence:?}"
        );
        assert!(
            sequence.contains(CURSOR_KEYS_NORMAL),
            "restore sequence should restore normal cursor key mode: {sequence:?}"
        );
        assert!(
            sequence.contains(KEYPAD_NORMAL),
            "restore sequence should restore normal keypad mode: {sequence:?}"
        );
        assert!(
            sequence.contains(ANSI_RESET),
            "restore sequence should reset terminal styling: {sequence:?}"
        );
        assert!(
            sequence.ends_with(ALT_SCREEN_EXIT),
            "restore sequence should leave the alternate screen last: {sequence:?}"
        );
    }

    #[test]
    fn ensure_parent_directory_exists_ignores_relative_files_without_parent() {
        let path = Path::new("transcript.txt");
        let result = ensure_parent_directory_exists(path);

        assert!(result.is_ok());
    }

    #[test]
    fn render_surface_to_string_draws_ratatui_panels() {
        let rendered = render_surface_to_string(
            &sample_surface_state(),
            &sample_render_data(),
            Rect::new(0, 0, 120, 32),
        );

        assert!(rendered.contains("loongclaw / chat"), "{rendered}");
        assert!(rendered.contains("transcript"), "{rendered}");
        assert!(rendered.contains("control deck"), "{rendered}");
        assert!(rendered.contains("compose"), "{rendered}");
        assert!(rendered.contains("controls"), "{rendered}");
        assert!(rendered.contains("OpenAI / gpt-5.4"), "{rendered}");
    }

    #[test]
    fn render_surface_to_string_renders_command_menu_overlay() {
        let mut state = sample_surface_state();
        state.command_palette = Some(CommandPaletteState {
            selected: 0,
            query: "help".to_owned(),
        });

        let rendered =
            render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

        assert!(rendered.contains("command menu"), "{rendered}");
        assert!(rendered.contains("/help"), "{rendered}");
    }

    #[test]
    fn render_surface_to_string_renders_welcome_overlay() {
        let mut state = sample_surface_state();
        state.overlay = Some(SurfaceOverlay::Welcome {
            screen: TuiScreenSpec {
                header_style: TuiHeaderStyle::Compact,
                subtitle: Some("interactive chat".to_owned()),
                title: Some("operator cockpit ready".to_owned()),
                progress_line: None,
                intro_lines: vec!["Start with a first answer.".to_owned()],
                sections: Vec::new(),
                choices: Vec::new(),
                footer_lines: vec!["Type to begin.".to_owned()],
            },
        });

        let rendered =
            render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

        assert!(rendered.contains("welcome"), "{rendered}");
        assert!(rendered.contains("operator cockpit ready"), "{rendered}");
        assert!(
            rendered.contains("Start with a first answer."),
            "{rendered}"
        );
    }

    #[test]
    fn render_surface_to_string_surfaces_review_tab_context() {
        let mut state = sample_surface_state();
        state.sidebar_tab = SidebarTab::Review;
        state.last_approval = Some(ApprovalSurfaceSummary {
            title: "tool approval".to_owned(),
            subtitle: Some("approval pending".to_owned()),
            request_items: vec!["tool: shell.exec".to_owned()],
            rationale_lines: vec!["Needs confirmation before continuing.".to_owned()],
            choice_lines: vec!["1: approve".to_owned(), "2: reject".to_owned()],
            footer_lines: vec!["Reply with 1 or 2".to_owned()],
        });
        let mut render_data = sample_render_data();
        render_data.sidebar_tab = SidebarTab::Review;
        render_data.sidebar_lines = vec![
            "approval: tool approval".to_owned(),
            "mode: approval pending".to_owned(),
            "request".to_owned(),
            "tool: shell.exec".to_owned(),
            "reason".to_owned(),
            "Needs confirmation before continuing.".to_owned(),
        ];

        let rendered = render_surface_to_string(&state, &render_data, Rect::new(0, 0, 120, 32));

        assert!(rendered.contains("approval: tool approval"), "{rendered}");
        assert!(rendered.contains("tool approval"), "{rendered}");
        assert!(rendered.contains("Needs confirmation"), "{rendered}");
    }

    #[test]
    fn approval_queue_item_summary_formats_list_and_detail_lines() {
        let item = ApprovalQueueItemSummary {
            approval_request_id: "apr_123".to_owned(),
            status: "pending".to_owned(),
            tool_name: "shell.exec".to_owned(),
            turn_id: "turn_9".to_owned(),
            requested_at: 42,
            reason: Some("governed tool requires approval".to_owned()),
            rule_id: Some("approval-visible".to_owned()),
            last_error: Some("still waiting".to_owned()),
        };

        assert!(item.list_line().contains("apr_123"));
        let detail = item.detail_lines().join("\n");
        assert!(detail.contains("approval_request_id=apr_123"));
        assert!(detail.contains("tool_name=shell.exec"));
        assert!(detail.contains("rule_id=approval-visible"));
        assert!(detail.contains("last_error=still waiting"));
    }

    #[test]
    fn worker_queue_item_summary_formats_list_and_detail_lines() {
        let item = WorkerQueueItemSummary {
            session_id: "child-1".to_owned(),
            label: "worker: lint".to_owned(),
            state: "running".to_owned(),
            kind: "delegate_child".to_owned(),
            parent_session_id: Some("root-session".to_owned()),
            turn_count: 3,
            updated_at: 77,
            last_error: Some("still working".to_owned()),
        };

        assert!(item.list_line().contains("worker: lint"));
        let detail = item.detail_lines().join("\n");
        assert!(detail.contains("session_id=child-1"));
        assert!(detail.contains("parent_session_id=root-session"));
        assert!(detail.contains("turn_count=3"));
        assert!(detail.contains("last_error=still working"));
    }

    #[test]
    fn render_surface_to_string_surfaces_worker_tab_context() {
        let mut state = sample_surface_state();
        state.sidebar_tab = SidebarTab::Workers;
        let mut render_data = sample_render_data();
        render_data.sidebar_tab = SidebarTab::Workers;
        render_data.sidebar_lines = vec![
            "worker sessions: 1".to_owned(),
            "worker: lint state=running kind=delegate_child turns=3".to_owned(),
        ];

        let rendered = render_surface_to_string(&state, &render_data, Rect::new(0, 0, 120, 32));

        assert!(rendered.contains("worker sessions: 1"), "{rendered}");
        assert!(rendered.contains("worker: lint"), "{rendered}");
        assert!(rendered.contains("delegate_child"), "{rendered}");
    }

    #[test]
    fn render_surface_to_string_surfaces_mission_overlay() {
        let mut state = sample_surface_state();
        state.overlay = Some(SurfaceOverlay::MissionControl {
            lines: vec![
                "scope: default".to_owned(),
                "lanes: sessions=2 · roots=1 · delegates=1 · approvals=1".to_owned(),
                "controls".to_owned(),
                "S sessions · W workers · R approval queue".to_owned(),
            ],
        });

        let rendered =
            render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

        assert!(rendered.contains("mission control"), "{rendered}");
        assert!(rendered.contains("lanes: sessions=2"), "{rendered}");
        assert!(rendered.contains("S sessions"), "{rendered}");
    }
}
