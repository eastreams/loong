use super::*;

#[derive(Clone)]
pub(super) struct SurfaceEntry {
    pub(super) lines: Vec<String>,
}

#[derive(Clone, Default)]
pub(super) struct SurfaceState {
    pub(super) startup_summary: Option<ops::CliChatStartupSummary>,
    pub(super) active_provider_label: String,
    pub(super) session_title_override: Option<String>,
    pub(super) last_approval: Option<ApprovalSurfaceSummary>,
    pub(super) transcript: Vec<SurfaceEntry>,
    pub(super) composer: String,
    pub(super) composer_cursor: usize,
    pub(super) history: Vec<String>,
    pub(super) history_index: Option<usize>,
    pub(super) scroll_offset: usize,
    pub(super) sticky_bottom: bool,
    pub(super) selected_entry: Option<usize>,
    pub(super) focus: SurfaceFocus,
    pub(super) sidebar_visible: bool,
    pub(super) sidebar_tab: SidebarTab,
    pub(super) command_palette: Option<CommandPaletteState>,
    pub(super) overlay: Option<SurfaceOverlay>,
    pub(super) live: LiveSurfaceModel,
    pub(super) footer_notice: String,
    pub(super) pending_turn: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum SidebarTab {
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
    pub(super) fn title(self) -> &'static str {
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

    pub(super) fn next(self) -> Self {
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

    pub(super) fn previous(self) -> Self {
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
pub(super) struct CommandPaletteState {
    pub(super) selected: usize,
    pub(super) query: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum SurfaceFocus {
    Transcript,
    #[default]
    Composer,
    Sidebar,
    CommandPalette,
}

#[derive(Clone, Debug)]
pub(super) enum SurfaceOverlay {
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
pub(super) enum OverlayInputKind {
    RenameSession,
    ExportTranscript,
}

impl SurfaceFocus {
    pub(super) fn next(self, sidebar_visible: bool, palette_open: bool) -> Self {
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

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Composer => "composer",
            Self::Sidebar => "sidebar",
            Self::CommandPalette => "palette",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandPaletteAction {
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
    pub(super) fn items() -> &'static [(&'static str, &'static str, Self)] {
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

pub(super) fn filtered_command_palette_items(
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
pub(super) struct LiveSurfaceModel {
    pub(super) snapshot: Option<CliChatLiveSurfaceSnapshot>,
    pub(super) state: CliChatLiveSurfaceState,
    pub(super) last_assistant_preview: Option<String>,
    pub(super) last_phase_label: String,
}
